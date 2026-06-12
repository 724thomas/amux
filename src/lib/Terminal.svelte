<script lang="ts">
  // Hosts one xterm.js instance bound to one engine pane.
  import { onMount } from "svelte";
  import { Terminal } from "@xterm/xterm";
  import { FitAddon } from "@xterm/addon-fit";
  import { WebglAddon } from "@xterm/addon-webgl";
  import { Unicode11Addon } from "@xterm/addon-unicode11";
  import { WebLinksAddon } from "@xterm/addon-web-links";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { readText, writeText } from "@tauri-apps/plugin-clipboard-manager";
  import "@xterm/xterm/css/xterm.css";
  import { writePane, resizePane, subscribePane, type PaneId } from "./ipc";
  import { handleKey } from "./keymap";
  import { adjustFontSize, settings } from "./settings.svelte";
  import { paneInfo } from "./state.svelte";

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

  // kitty keyboard protocol state of the app in this pane (e.g. Claude Code).
  // When active, the app wants shift+arrows itself (input highlighting) and
  // the CSI-u encodings for Esc / modified Enter.
  const kitty = $derived(paneInfo(pane)?.meta.kitty_keyboard === true);

  // Terminal-level keyboard selection (for apps without kitty protocol):
  // shift+arrow extends a visual selection anchored at the cursor.
  let kbSel: { anchor: number; extent: number } | null = null;

  function extendKeyboardSelection(key: string) {
    const buf = term.buffer.active;
    const cols = term.cols;
    if (!kbSel) {
      const cursor = (buf.baseY + buf.cursorY) * cols + buf.cursorX;
      kbSel = { anchor: cursor, extent: cursor };
    }
    const delta =
      key === "ArrowLeft" ? -1 : key === "ArrowRight" ? 1 : key === "ArrowUp" ? -cols : cols;
    const max = buf.length * cols - 1;
    kbSel.extent = Math.min(max, Math.max(0, kbSel.extent + delta));
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
          void pasteClipboard();
          return false;
        }
      }
      if (e.type === "keydown" && e.ctrlKey && !e.shiftKey && !e.altKey) {
        // Ctrl+C copies when text is selected; otherwise it stays SIGINT.
        if (e.code === "KeyC" && term.hasSelection()) {
          void copySelection().then(() => term.clearSelection());
          return false;
        }
        // Ctrl+V always pastes (readline's literal-next is Ctrl+Shift+V
        // territory for the rare user who needs it... which we also use
        // for paste, so literal-next is effectively retired here).
        if (e.code === "KeyV") {
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
    const doFit = () => {
      // Hidden workspaces report 0×0; fitting then would shrink the PTY to
      // a few columns and garble every TUI in the pane. Skip until visible.
      if (host.clientWidth < 20 || host.clientHeight < 20) return;
      fit.fit();
      if (term.cols >= 2 && term.rows >= 2) {
        void resizePane(pane, term.cols, term.rows);
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

    return () => {
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

  async function menuAction(action: "copy" | "paste" | "selectAll" | "clear") {
    menu = null;
    switch (action) {
      case "copy":
        await copySelection();
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
></div>

{#if menu}
  <div class="ctx-menu" style="left: {menu.x}px; top: {menu.y}px">
    <button onclick={() => menuAction("copy")} disabled={!term?.hasSelection()}>복사</button>
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
</style>
