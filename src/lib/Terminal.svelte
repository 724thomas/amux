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
  import { paneInfo, registerTermFocus } from "./state.svelte";

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
      theme: { background: "#16161e" },
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

    const channel = subscribePane(pane, (chunk) => term.write(chunk));
    term.onData((data) => {
      clearKeyboardSelection();
      void writePane(pane, data);
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
    background: #16161e;
  }
  .ctx-menu {
    position: fixed;
    z-index: 1000;
    display: flex;
    flex-direction: column;
    min-width: 10rem;
    padding: 0.25rem;
    background: #1f2335;
    border: 1px solid #3b4261;
    border-radius: 6px;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.5);
  }
  .ctx-menu button {
    padding: 0.4rem 0.75rem;
    text-align: left;
    color: #c0caf5;
    background: none;
    border: none;
    border-radius: 4px;
    cursor: pointer;
    font-size: 0.85rem;
  }
  .ctx-menu button:hover:not(:disabled) {
    background: #3b4261;
  }
  .ctx-menu button:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .ctx-menu hr {
    margin: 0.25rem 0.5rem;
    border: none;
    border-top: 1px solid #3b4261;
  }
  .search-bar {
    position: absolute;
    top: 4px;
    right: 110px;
    z-index: 20;
    display: flex;
    gap: 2px;
    padding: 3px;
    background: #1f2335;
    border: 1px solid #3b4261;
    border-radius: 6px;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.5);
  }
  .search-bar input {
    width: 11rem;
    padding: 2px 6px;
    font-size: 0.8rem;
    color: #c0caf5;
    background: #16161e;
    border: 1px solid #3b4261;
    border-radius: 4px;
  }
  .search-bar input:focus {
    outline: none;
    border-color: #7aa2f7;
  }
  .search-bar button {
    width: 22px;
    color: #c0caf5;
    background: none;
    border: none;
    border-radius: 4px;
    cursor: pointer;
  }
  .search-bar button:hover {
    background: #3b4261;
  }
</style>
