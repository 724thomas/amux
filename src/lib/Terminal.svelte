<script lang="ts">
  // Hosts one xterm.js instance bound to one engine pane.
  import { onMount } from "svelte";
  import { Terminal } from "@xterm/xterm";
  import { FitAddon } from "@xterm/addon-fit";
  import { WebglAddon } from "@xterm/addon-webgl";
  import { Unicode11Addon } from "@xterm/addon-unicode11";
  import { WebLinksAddon } from "@xterm/addon-web-links";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import "@xterm/xterm/css/xterm.css";
  import { writePane, resizePane, subscribePane, type PaneId } from "./ipc";
  import { handleKey } from "./keymap";
  import { adjustFontSize, settings } from "./settings.svelte";

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
    term.attachCustomKeyEventHandler((e) => !handleKey(e));
    term.open(host);
    try {
      const webgl = new WebglAddon();
      webgl.onContextLoss(() => webgl.dispose()); // falls back to DOM renderer
      term.loadAddon(webgl);
    } catch {
      // WebKitGTK without a usable WebGL context: DOM renderer is fine.
    }

    const channel = subscribePane(pane, (chunk) => term.write(chunk));
    term.onData((data) => void writePane(pane, data));

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

  async function menuAction(action: "copy" | "paste" | "selectAll" | "clear") {
    menu = null;
    switch (action) {
      case "copy": {
        const sel = term.getSelection();
        if (sel) await navigator.clipboard.writeText(sel);
        break;
      }
      case "paste": {
        const text = await navigator.clipboard.readText();
        if (text) await writePane(pane, text);
        break;
      }
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
