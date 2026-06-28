<script lang="ts">
  // Hosts one xterm.js instance bound to one engine pane.
  import { onMount } from "svelte";
  import { Terminal } from "@xterm/xterm";
  import { FitAddon } from "@xterm/addon-fit";
  import { WebglAddon } from "@xterm/addon-webgl";
  import { Unicode11Addon } from "@xterm/addon-unicode11";
  import { WebLinksAddon } from "@xterm/addon-web-links";
  import { SearchAddon } from "@xterm/addon-search";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { readText, writeText } from "@tauri-apps/plugin-clipboard-manager";
  import "@xterm/xterm/css/xterm.css";
  import { writePane, resizePane, subscribePane, type PaneId } from "./ipc";
  import { handleKey } from "./keymap";
  import { adjustFontSize, settings } from "./settings.svelte";
  import { themeById } from "./themes";
  import { paneInfo, registerTermFocus, broadcast, broadcastTargets } from "./state.svelte";

  export interface MenuAction {
    label: string;
    run: () => void;
  }

  let {
    pane,
    focused = true,
    extraActions = [],
  }: { pane: PaneId; focused?: boolean; extraActions?: MenuAction[] } = $props();

  let host: HTMLDivElement;
  let menu = $state<{ x: number; y: number } | null>(null);
  let term = $state<Terminal>()!;
  let refit: (() => void) | undefined;

  // ── Activity widgets ──────────────────────────────────────────────────
  // Two visualizers of the SAME signal — this pane's output byte-rate (never
  // its content): an oscilloscope waveform paired with an Arc Reactor core,
  // top-right. The bound canvases + accent color live here; the byte counter
  // and rAF loop live in onMount, next to the chunk stream.
  let waveCanvas: HTMLCanvasElement;
  let arcCanvas: HTMLCanvasElement;
  let waveColor = "#7aa2f7"; // accent → wave + arc strokes
  let waveRGB = "122, 162, 247"; // accent as "r, g, b" for rgba() fills
  function readAccent() {
    let h = getComputedStyle(document.documentElement)
      .getPropertyValue("--accent")
      .trim()
      .replace("#", "");
    if (h.length === 3) h = h.split("").map((c) => c + c).join("");
    if (h.length < 6) return;
    const r = parseInt(h.slice(0, 2), 16);
    const g = parseInt(h.slice(2, 4), 16);
    const b = parseInt(h.slice(4, 6), 16);
    if (Number.isNaN(r) || Number.isNaN(g) || Number.isNaN(b)) return;
    waveColor = "#" + h.slice(0, 6);
    waveRGB = `${r}, ${g}, ${b}`;
  }

  let search: SearchAddon;
  let searchOpen = $state(false);
  let searchQuery = $state("");

  const searchOptions = { decorations: { matchOverviewRuler: "#7aa2f7", activeMatchColorOverviewRuler: "#ff9e64" } };

  function closeSearch() {
    searchOpen = false;
    searchQuery = "";
    search?.clearDecorations();
    term?.clearSelection();
    term?.focus();
  }

  // kitty keyboard protocol state of the app in this pane (e.g. Claude Code).
  // When active, the app wants shift+arrows itself (input highlighting) and
  // the CSI-u encodings for Esc / modified Enter.
  const kitty = $derived(paneInfo(pane)?.meta.kitty_keyboard === true);

  // Terminal-level keyboard selection: shift+arrow extends a visual
  // selection anchored at the cursor. `chars` counts horizontal presses
  // (negative = leftward) so cut can translate to Backspace/Delete;
  // vertical selections can only be copied, not cut.
  let kbSel: { anchor: number; extent: number; chars: number; vertical: boolean } | null = null;

  function extendKeyboardSelection(key: string) {
    const buf = term.buffer.active;
    const cols = term.cols;
    if (!kbSel) {
      const cursor = (buf.baseY + buf.cursorY) * cols + buf.cursorX;
      kbSel = { anchor: cursor, extent: cursor, chars: 0, vertical: false };
    }
    if (key === "ArrowUp" || key === "ArrowDown") {
      kbSel.vertical = true;
      kbSel.extent += key === "ArrowUp" ? -cols : cols;
    } else {
      const dir = key === "ArrowLeft" ? -1 : 1;
      kbSel.chars += dir;
      kbSel.extent += dir;
      // Step over wide-char spacer cells so CJK glyphs stay whole.
      const line = buf.getLine(Math.floor(kbSel.extent / cols));
      if (line?.getCell(kbSel.extent % cols)?.getWidth() === 0) {
        kbSel.extent += dir;
      }
    }
    const max = buf.length * cols - 1;
    kbSel.extent = Math.min(max, Math.max(0, kbSel.extent));
    const start = Math.min(kbSel.anchor, kbSel.extent);
    const end = Math.max(kbSel.anchor, kbSel.extent);
    term.select(start % cols, Math.floor(start / cols), Math.max(1, end - start));
  }

  function clearKeyboardSelection() {
    if (kbSel) {
      kbSel = null;
      term.clearSelection();
    }
  }

  /// Cut: copy, then — for horizontal keyboard selections — actually delete
  /// the characters from the app's input (Backspace for leftward selections,
  /// Delete for rightward). Mouse/vertical selections copy only.
  async function cutSelection() {
    if (!term.hasSelection()) return;
    await copySelection();
    if (kbSel && !kbSel.vertical && kbSel.chars !== 0) {
      const n = Math.abs(kbSel.chars);
      const seq = kbSel.chars < 0 ? "\x7f".repeat(n) : "\x1b[3~".repeat(n);
      await writePane(pane, seq);
    }
    kbSel = null;
    term.clearSelection();
  }

  onMount(() => {
    term = new Terminal({
      allowProposedApi: true,
      scrollback: 10_000,
      fontFamily: "monospace",
      fontSize: settings.fontSize,
      theme: themeById(settings.theme).term,
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.loadAddon(new Unicode11Addon());
    term.unicode.activeVersion = "11";
    search = new SearchAddon();
    term.loadAddon(search);
    term.loadAddon(
      new WebLinksAddon((e, uri) => {
        if (e.ctrlKey) void openUrl(uri);
      }),
    );
    // App shortcuts (split/navigate/...) win over the terminal; everything
    // else (Ctrl+C, Tab, F-keys...) flows through to the shell untouched.
    term.attachCustomKeyEventHandler((e) => {
      // Shift+Enter → 줄바꿈: kitty 모드 앱(Claude Code)에는 CSI-u 인코딩,
      // 그 외에는 ESC+CR (iTerm2 /terminal-setup과 동일한 매핑).
      if (
        e.type === "keydown" &&
        e.key === "Enter" &&
        e.shiftKey &&
        !e.ctrlKey &&
        !e.altKey
      ) {
        void writePane(pane, kitty ? "\x1b[13;2u" : "\x1b\r");
        return false;
      }
      // kitty 모드에서 plain Esc는 CSI 27u로 보고해야 함 (프로토콜 규약).
      if (
        e.type === "keydown" &&
        e.key === "Escape" &&
        kitty &&
        !e.ctrlKey &&
        !e.altKey &&
        !e.shiftKey
      ) {
        void writePane(pane, "\x1b[27u");
        return false;
      }
      // Shift+방향키 → 터미널 키보드 선택 (커서 기준 하이라이트 확장,
      // copy-on-select로 자동 복사). Claude Code는 입력창 키보드 선택이
      // 아직 없으므로(anthropics/claude-code#23396) 항상 터미널이 갖는다.
      if (
        e.type === "keydown" &&
        e.shiftKey &&
        !e.ctrlKey &&
        !e.altKey &&
        e.key.startsWith("Arrow")
      ) {
        extendKeyboardSelection(e.key);
        return false;
      }
      if (e.type === "keydown" && e.ctrlKey && e.shiftKey && !e.altKey) {
        // Terminal-convention clipboard keys, via the Rust clipboard
        // (navigator.clipboard is unreliable in WebKitGTK).
        if (e.code === "KeyC" && term.hasSelection()) {
          void copySelection();
          return false;
        }
        if (e.code === "KeyV") {
          // 브라우저 네이티브 paste가 한 번 더 붙는 것 방지
          e.preventDefault();
          void pasteClipboard();
          return false;
        }
        // 스크롤백 검색
        if (e.code === "KeyF") {
          searchOpen = true;
          return false;
        }
      }
      if (e.type === "keydown" && e.ctrlKey && !e.shiftKey && !e.altKey) {
        // Ctrl+C copies when text is selected; otherwise it stays SIGINT.
        if (e.code === "KeyC" && term.hasSelection()) {
          void copySelection().then(() => {
            kbSel = null;
            term.clearSelection();
          });
          return false;
        }
        // Ctrl+X 잘라내기: 복사 후, 키보드 선택(가로)은 입력에서 실제 삭제.
        // 선택이 없으면 nano/emacs 등이 쓰는 원래 Ctrl+X로 동작.
        if (e.code === "KeyX" && term.hasSelection()) {
          void cutSelection();
          return false;
        }
        // Ctrl+V always pastes (readline's literal-next is Ctrl+Shift+V
        // territory for the rare user who needs it... which we also use
        // for paste, so literal-next is effectively retired here).
        if (e.code === "KeyV") {
          // 브라우저 네이티브 paste가 한 번 더 붙는 것 방지
          e.preventDefault();
          void pasteClipboard();
          return false;
        }
      }
      return !handleKey(e);
    });
    // Linux terminal convention: selecting text copies it.
    let selectionTimer: ReturnType<typeof setTimeout> | undefined;
    term.onSelectionChange(() => {
      clearTimeout(selectionTimer);
      selectionTimer = setTimeout(() => {
        if (term.hasSelection()) void writeText(term.getSelection());
      }, 150);
    });
    term.open(host);
    try {
      const webgl = new WebglAddon();
      webgl.onContextLoss(() => webgl.dispose()); // falls back to DOM renderer
      term.loadAddon(webgl);
    } catch {
      // WebKitGTK without a usable WebGL context: DOM renderer is fine.
    }

    // ── Thinking Waveform engine ──────────────────────────────────────
    // Each frame folds the bytes seen since the last frame into one smoothed
    // sample (fast attack, slow release) and pushes it into a fixed ~3s
    // history stretched across the strip. That 3s window doubles as a linger:
    // a TUI's spinner redrawing ~1–2×/s keeps the loop awake, so it reads as a
    // continuous ripple, not a wake/sleep strobe. Once the window is genuinely
    // flat the loop sleeps and clears — idle panes draw nothing, cost nothing.
    const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const HISTORY = 180; // samples ≈ 3s @ 60fps — the visible time window
    const RELEASE = 0.84; // per-frame decay of the smoothed level
    const SCALE = 600; // bytes/frame → ~full height (sqrt-compressed)
    const history: number[] = []; // oldest→newest; newest drawn at the right edge
    let bytesSinceFrame = 0;
    let smoothed = 0;
    let waveRaf = 0;
    let waveRunning = false;
    let arcAngle = 0; // Arc Reactor sweep rotation (persists across frames)

    const levelFromBytes = (bytes: number) =>
      bytes <= 0 ? 0 : Math.min(1, Math.sqrt(bytes / SCALE));
    // Gamma < 1 lifts light activity (a spinner's trickle) so it still reads
    // boldly, while heavy streaming still tops out near full.
    const GAIN = 0.7;
    const shape = (v: number) => Math.pow(v, GAIN);
    const recentPeak = () => {
      let p = 0;
      for (let i = 0; i < history.length; i++) if (history[i] > p) p = history[i];
      return p;
    };
    // Size each canvas to its CSS box (dpr-aware) and clear it. Returns null
    // when it's hidden/zero-sized so the caller bails.
    function prep(c: HTMLCanvasElement) {
      const w = c.clientWidth,
        h = c.clientHeight;
      if (w <= 0 || h <= 0) return null;
      const dpr = window.devicePixelRatio || 1;
      const nw = Math.round(w * dpr),
        nh = Math.round(h * dpr);
      if (c.width !== nw || c.height !== nh) {
        c.width = nw;
        c.height = nh;
      }
      const ctx = c.getContext("2d");
      if (!ctx) return null;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, w, h);
      return { ctx, w, h };
    }

    // Waveform — a mirrored oscilloscope of the last ~3s of activity.
    function drawWave() {
      const p = prep(waveCanvas);
      if (!p) return;
      const { ctx, w, h } = p;
      const n = history.length;
      if (n < 2) return;
      const mid = h / 2;
      const amp = mid - 2;
      const step = w / (HISTORY - 1);
      const x0 = w - (n - 1) * step; // right-align the newest sample
      const yTop = (v: number) => mid - shape(v) * amp;
      const yBot = (v: number) => mid + shape(v) * amp;
      const peak = shape(recentPeak());

      ctx.beginPath();
      ctx.moveTo(x0, yTop(history[0]));
      for (let i = 1; i < n; i++) ctx.lineTo(x0 + i * step, yTop(history[i]));
      for (let i = n - 1; i >= 0; i--) ctx.lineTo(x0 + i * step, yBot(history[i]));
      ctx.closePath();
      const grad = ctx.createLinearGradient(0, 0, 0, h);
      grad.addColorStop(0, `rgba(${waveRGB}, 0)`);
      grad.addColorStop(0.5, `rgba(${waveRGB}, ${0.55 * peak})`);
      grad.addColorStop(1, `rgba(${waveRGB}, 0)`);
      ctx.fillStyle = grad;
      ctx.fill();

      ctx.beginPath();
      ctx.moveTo(x0, yTop(history[0]));
      for (let i = 1; i < n; i++) ctx.lineTo(x0 + i * step, yTop(history[i]));
      ctx.strokeStyle = waveColor;
      ctx.lineWidth = 1.5;
      ctx.globalAlpha = 0.25 + 0.7 * peak;
      ctx.shadowColor = waveColor;
      ctx.shadowBlur = 8;
      ctx.stroke();
      ctx.globalAlpha = 1;
      ctx.shadowBlur = 0;
    }

    // Arc Reactor — a ring with a rotating sweep + core, Iron-Man HUD style.
    function drawArc() {
      const p = prep(arcCanvas);
      if (!p) return;
      const { ctx, w, h } = p;
      const lvl = shape(smoothed);
      const pk = shape(recentPeak());
      const cx = w / 2,
        cy = h / 2;
      const R = Math.min(w, h) / 2 - 4;
      arcAngle = (arcAngle + 0.04 + lvl * 0.5) % (Math.PI * 2); // spin ∝ throughput
      ctx.lineWidth = 2 + pk * 2.5; // base ring
      ctx.strokeStyle = `rgba(${waveRGB}, ${0.18 + 0.32 * pk})`;
      ctx.beginPath();
      ctx.arc(cx, cy, R, 0, Math.PI * 2);
      ctx.stroke();
      ctx.strokeStyle = waveColor; // bright rotating sweep (~90°)
      ctx.globalAlpha = 0.5 + 0.5 * pk;
      ctx.shadowColor = waveColor;
      ctx.shadowBlur = 4 + 8 * pk;
      ctx.beginPath();
      ctx.arc(cx, cy, R, arcAngle, arcAngle + Math.PI * 0.5);
      ctx.stroke();
      ctx.shadowBlur = 0;
      ctx.globalAlpha = 1;
      const cr = Math.max(0.2, 1.5 + lvl * (R - 3)); // core glow
      const rg = ctx.createRadialGradient(cx, cy, 0, cx, cy, cr);
      rg.addColorStop(0, `rgba(${waveRGB}, ${0.6 + 0.4 * lvl})`);
      rg.addColorStop(1, `rgba(${waveRGB}, 0)`);
      ctx.fillStyle = rg;
      ctx.beginPath();
      ctx.arc(cx, cy, cr, 0, Math.PI * 2);
      ctx.fill();
    }

    function waveTick() {
      waveRaf = 0;
      const bytes = bytesSinceFrame;
      bytesSinceFrame = 0;
      smoothed = Math.max(levelFromBytes(bytes), smoothed * RELEASE);
      if (smoothed < 0.004) smoothed = 0;
      history.push(smoothed);
      if (history.length > HISTORY) history.shift();
      if (host.clientWidth >= 20 && host.clientHeight >= 20) {
        drawWave();
        drawArc();
      }
      // Keep animating until the last pulse has shifted out of the window.
      if (smoothed > 0 || history.some((v) => v > 0.004)) {
        waveRaf = requestAnimationFrame(waveTick);
      } else {
        waveRunning = false;
        history.length = 0;
        for (const c of [waveCanvas, arcCanvas])
          c?.getContext("2d")?.clearRect(0, 0, c.width, c.height);
      }
    }

    function waveWake() {
      if (waveRunning || reduceMotion) return;
      waveRunning = true;
      waveRaf = requestAnimationFrame(waveTick);
    }

    readAccent();
    const channel = subscribePane(pane, (chunk) => {
      term.write(chunk);
      bytesSinceFrame += chunk.length; // byte count only — never the content
      waveWake();
    });
    term.onData((data) => {
      clearKeyboardSelection();
      void writePane(pane, data);
      // Broadcast (synchronize-panes): mirror this pane's input to every other
      // live pane in the workspace. Only the focused pane originates; writePane
      // feeds the PTY (not xterm.onData), so mirrored panes never echo back.
      if (broadcast.on && focused) {
        for (const target of broadcastTargets(pane)) void writePane(target, data);
      }
    });

    let resizeRaf = 0;
    let wasHidden = true;
    const doFit = () => {
      // Hidden workspaces report 0×0; fitting then would shrink the PTY to
      // a few columns and garble every TUI in the pane. Skip until visible.
      if (host.clientWidth < 20 || host.clientHeight < 20) {
        wasHidden = true;
        return;
      }
      fit.fit();
      if (term.cols >= 2 && term.rows >= 2) {
        void resizePane(pane, term.cols, term.rows);
      }
      if (wasHidden) {
        // Coming back from display:none the canvas is stale/blank and a
        // same-size fit() is a no-op, so nothing would repaint it.
        wasHidden = false;
        term.refresh(0, term.rows - 1);
      }
    };
    refit = doFit;
    const observer = new ResizeObserver(() => {
      cancelAnimationFrame(resizeRaf);
      resizeRaf = requestAnimationFrame(doFit);
    });
    observer.observe(host);
    doFit();
    if (focused) term.focus();
    const unregisterFocus = registerTermFocus(pane, () => term.focus());

    return () => {
      unregisterFocus();
      observer.disconnect();
      cancelAnimationFrame(waveRaf);
      channel.onmessage = () => {};
      term.dispose();
    };
  });

  $effect(() => {
    if (focused && term) term.focus();
  });

  // Live font-size changes: update xterm, then refit cols/rows to the host.
  $effect(() => {
    const size = settings.fontSize;
    if (term && term.options.fontSize !== size) {
      term.options.fontSize = size;
      refit?.();
    }
  });

  // Live theme changes.
  $effect(() => {
    const theme = themeById(settings.theme).term;
    if (term) term.options.theme = theme;
  });

  // Thinking Waveform crest follows the active theme; re-read after the next
  // frame so the theme's CSS vars are already applied to :root.
  $effect(() => {
    void settings.theme;
    requestAnimationFrame(readAccent);
  });

  async function copySelection() {
    const sel = term.getSelection();
    if (sel) await writeText(sel);
  }

  async function pasteClipboard() {
    try {
      const text = await readText();
      // term.paste() honors bracketed-paste mode (vim, fzf, modern shells)
      // and feeds onData → PTY.
      if (text) term.paste(text);
    } catch {
      // Clipboard empty or unreadable — nothing to paste.
    }
  }

  async function menuAction(action: "copy" | "cut" | "paste" | "selectAll" | "clear") {
    menu = null;
    switch (action) {
      case "copy":
        await copySelection();
        break;
      case "cut":
        await cutSelection();
        break;
      case "paste":
        await pasteClipboard();
        break;
      case "selectAll":
        term.selectAll();
        break;
      case "clear":
        term.clear();
        break;
    }
    term.focus();
  }
</script>

<svelte:window onclick={() => (menu = null)} />

<div
  class="terminal-host"
  role="application"
  bind:this={host}
  oncontextmenu={(e) => {
    e.preventDefault();
    menu = { x: e.clientX, y: e.clientY };
  }}
  onwheel={(e) => {
    // Ctrl+wheel zooms the font, like GNOME Terminal.
    if (e.ctrlKey) {
      e.preventDefault();
      adjustFontSize(e.deltaY < 0 ? 1 : -1);
    }
  }}
  onauxclick={(e) => {
    // Middle-click pastes, like every Linux terminal.
    if (e.button === 1) {
      e.preventDefault();
      void pasteClipboard();
    }
  }}
  onpastecapture={(e) => {
    // 네이티브 paste 경로 차단 — 붙여넣기는 항상 우리(Rust 클립보드)
    // 경로 하나로만 들어와 이중 붙여넣기를 방지.
    e.preventDefault();
    e.stopPropagation();
  }}
></div>

<div class="wave-lab" aria-hidden="true">
  <canvas class="wl wave" bind:this={waveCanvas}></canvas>
  <canvas class="wl arc" bind:this={arcCanvas}></canvas>
</div>

{#if searchOpen}
  <div class="search-bar">
    <!-- svelte-ignore a11y_autofocus -->
    <input
      autofocus
      placeholder="검색…"
      bind:value={searchQuery}
      oninput={() => search.findNext(searchQuery, { ...searchOptions, incremental: true })}
      onkeydown={(e) => {
        e.stopPropagation();
        if (e.key === "Enter" && e.shiftKey) search.findPrevious(searchQuery, searchOptions);
        else if (e.key === "Enter") search.findNext(searchQuery, searchOptions);
        else if (e.key === "Escape") closeSearch();
      }}
    />
    <button title="이전 (Shift+Enter)" onclick={() => search.findPrevious(searchQuery, searchOptions)}>↑</button>
    <button title="다음 (Enter)" onclick={() => search.findNext(searchQuery, searchOptions)}>↓</button>
    <button title="닫기 (Esc)" onclick={closeSearch}>✕</button>
  </div>
{/if}

{#if menu}
  <div class="ctx-menu" style="left: {menu.x}px; top: {menu.y}px">
    <button onclick={() => menuAction("copy")} disabled={!term?.hasSelection()}>복사</button>
    <button onclick={() => menuAction("cut")} disabled={!term?.hasSelection()}>잘라내기</button>
    <button onclick={() => menuAction("paste")}>붙여넣기</button>
    <button onclick={() => menuAction("selectAll")}>모두 선택</button>
    <button onclick={() => menuAction("clear")}>화면 지우기</button>
    {#if extraActions.length > 0}
      <hr />
      {#each extraActions as action (action.label)}
        <button
          onclick={() => {
            menu = null;
            action.run();
          }}>{action.label}</button
        >
      {/each}
    {/if}
  </div>
{/if}

<style>
  .terminal-host {
    width: 100%;
    height: 100%;
    background: var(--bg);
  }
  /* Activity widgets — the oscilloscope waveform (left) paired with the Arc
     Reactor core (right), in the pane's top-right corner. Above the terminal,
     below the hover toolbar/shockwave; never eat clicks. Both adapt in JS. */
  .wave-lab {
    position: absolute;
    top: 6px;
    right: 8px;
    z-index: 4;
    pointer-events: none;
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 8px;
  }
  .wl {
    display: block;
  }
  .wl.wave {
    width: 140px;
    height: 46px;
  }
  .wl.arc {
    width: 48px;
    height: 46px;
  }
  @media (prefers-reduced-motion: reduce) {
    .wave-lab {
      display: none;
    }
  }
  .ctx-menu {
    position: fixed;
    z-index: 1000;
    display: flex;
    flex-direction: column;
    min-width: 10rem;
    padding: 0.25rem;
    background: var(--surface-2);
    border: 1px solid var(--border-2);
    border-radius: 6px;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.5);
  }
  .ctx-menu button {
    padding: 0.4rem 0.75rem;
    text-align: left;
    color: var(--text);
    background: none;
    border: none;
    border-radius: 4px;
    cursor: pointer;
    font-size: 0.85rem;
  }
  .ctx-menu button:hover:not(:disabled) {
    background: var(--border-2);
  }
  .ctx-menu button:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .ctx-menu hr {
    margin: 0.25rem 0.5rem;
    border: none;
    border-top: 1px solid var(--border-2);
  }
  .search-bar {
    position: absolute;
    top: 4px;
    right: 110px;
    z-index: 20;
    display: flex;
    gap: 2px;
    padding: 3px;
    background: var(--surface-2);
    border: 1px solid var(--border-2);
    border-radius: 6px;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.5);
  }
  .search-bar input {
    width: 11rem;
    padding: 2px 6px;
    font-size: 0.8rem;
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--border-2);
    border-radius: 4px;
  }
  .search-bar input:focus {
    outline: none;
    border-color: var(--accent);
  }
  .search-bar button {
    width: 22px;
    color: var(--text);
    background: none;
    border: none;
    border-radius: 4px;
    cursor: pointer;
  }
  .search-bar button:hover {
    background: var(--border-2);
  }
</style>
