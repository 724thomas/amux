<script lang="ts">
  import { onMount } from "svelte";
  import Sidebar from "./lib/Sidebar.svelte";
  import SplitNode from "./lib/SplitNode.svelte";
  import Palette from "./lib/Palette.svelte";
  import Dashboard from "./lib/Dashboard.svelte";
  import { app, initState, broadcast, palette, dashboard, activeWorkspacePaneCount } from "./lib/state.svelte";
  import { handleKey } from "./lib/keymap";
  import { setSidebarWidth, settings } from "./lib/settings.svelte";
  import { themeById } from "./lib/themes";

  const snapshot = $derived(app.snapshot);
  const bcastCount = $derived(activeWorkspacePaneCount());

  let draggingSidebar = $state(false);

  onMount(() => {
    void initState();
  });

  // Theme: app chrome colors live as CSS variables on :root.
  $effect(() => {
    const { chrome } = themeById(settings.theme);
    for (const [key, value] of Object.entries(chrome)) {
      document.documentElement.style.setProperty(key, value);
    }
  });
</script>

<!-- Shortcuts also work when focus is outside any terminal. -->
<svelte:window
  onkeydown={(e) => {
    if (handleKey(e)) e.preventDefault();
  }}
/>

{#if broadcast.on}
  <button
    class="bcast-banner"
    onclick={() => (broadcast.on = false)}
    title="브로드캐스트 해제 (클릭 또는 Ctrl+Shift+B)"
  >
    <span class="bolt">⚡</span>
    BROADCAST — 입력이 {bcastCount}개 pane에 동시 전송됩니다
    <span class="hint">클릭 · Ctrl+Shift+B 해제</span>
  </button>
{/if}

{#if palette.open}
  <Palette />
{/if}

{#if dashboard.open}
  <div
    class="dash-overlay"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) dashboard.open = false;
    }}
  >
    <div class="dash-modal">
      <span class="hud-bk tl"></span>
      <span class="hud-bk tr"></span>
      <span class="hud-bk bl"></span>
      <span class="hud-bk br"></span>
      <span class="hud-scan"></span>
      <Dashboard dismiss={() => (dashboard.open = false)} />
    </div>
  </div>
{/if}

<div class="shell">
  <div class="sidebar-wrap" style="width: {settings.sidebarWidth}px">
    <Sidebar />
  </div>
  <div
    class="sidebar-resizer"
    class:dragging={draggingSidebar}
    role="separator"
    aria-orientation="vertical"
    onpointerdown={(e) => {
      draggingSidebar = true;
      (e.target as HTMLElement).setPointerCapture(e.pointerId);
    }}
    onpointermove={(e) => {
      if (draggingSidebar) setSidebarWidth(e.clientX);
    }}
    onpointerup={() => (draggingSidebar = false)}
    onpointercancel={() => (draggingSidebar = false)}
    ondblclick={() => setSidebarWidth(230)}
  ></div>
  <main class="main">
    <!-- Every workspace stays mounted so its terminals keep their xterm
         buffers; only the active one is displayed. -->
    {#each snapshot?.workspaces ?? [] as ws (ws.id)}
      <div class="workspace" class:hidden={ws.id !== snapshot?.active_workspace}>
        <SplitNode
          node={ws.layout}
          workspace={ws.id}
          activePane={ws.active_pane}
          visible={ws.id === snapshot?.active_workspace}
        />
      </div>
    {/each}
  </main>
</div>

<style>
  .shell {
    display: flex;
    width: 100vw;
    height: 100vh;
    background: var(--bg);
  }
  .sidebar-wrap {
    flex-shrink: 0;
    min-width: 0;
    display: flex;
  }
  .sidebar-resizer {
    flex: 0 0 4px;
    cursor: col-resize;
    background: var(--border);
    touch-action: none;
  }
  .sidebar-resizer:hover,
  .sidebar-resizer.dragging {
    background: var(--accent);
  }
  .main {
    position: relative;
    flex: 1;
    min-width: 0;
  }
  .workspace {
    position: absolute;
    inset: 0;
  }
  .workspace.hidden {
    display: none;
  }

  /* Dashboard — JARVIS-style holographic command overlay (Ctrl+Shift+A). */
  .dash-overlay {
    position: fixed;
    inset: 0;
    z-index: 9000;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 5vh 5vw;
    background:
      radial-gradient(
        ellipse at center,
        transparent 38%,
        color-mix(in srgb, var(--bg) 82%, #000) 100%
      ),
      color-mix(in srgb, var(--bg) 72%, transparent);
    animation: dash-fade 0.22s ease;
  }
  @keyframes dash-fade {
    from {
      opacity: 0;
    }
    to {
      opacity: 1;
    }
  }
  .dash-modal {
    position: relative;
    width: min(1180px, 100%);
    height: min(82vh, 100%);
    overflow: hidden;
    border-radius: 14px;
    background: linear-gradient(
      160deg,
      color-mix(in srgb, var(--info) 9%, var(--bg)) 0%,
      var(--bg) 58%
    );
    border: 1px solid color-mix(in srgb, var(--info) 55%, transparent);
    box-shadow:
      0 0 0 1px color-mix(in srgb, var(--accent) 22%, transparent),
      0 0 40px -6px color-mix(in srgb, var(--info) 50%, transparent),
      0 30px 80px rgba(0, 0, 0, 0.6),
      inset 0 0 70px -22px color-mix(in srgb, var(--info) 42%, transparent);
    animation: dash-materialize 0.34s cubic-bezier(0.2, 0.8, 0.2, 1);
  }
  @keyframes dash-materialize {
    from {
      opacity: 0;
      transform: scale(0.965) translateY(8px);
    }
    to {
      opacity: 1;
      transform: none;
    }
  }
  /* faint techno grid behind the board */
  .dash-modal::before {
    content: "";
    position: absolute;
    inset: 0;
    z-index: 0;
    pointer-events: none;
    background-image:
      linear-gradient(color-mix(in srgb, var(--info) 60%, transparent) 1px, transparent 1px),
      linear-gradient(90deg, color-mix(in srgb, var(--info) 60%, transparent) 1px, transparent 1px);
    background-size: 34px 34px;
    opacity: 0.05;
  }
  /* HUD corner brackets */
  .hud-bk {
    position: absolute;
    z-index: 3;
    width: 22px;
    height: 22px;
    border: 0 solid var(--info);
    pointer-events: none;
    filter: drop-shadow(0 0 4px color-mix(in srgb, var(--info) 70%, transparent));
  }
  .hud-bk.tl {
    top: 8px;
    left: 8px;
    border-top-width: 2px;
    border-left-width: 2px;
    border-top-left-radius: 6px;
  }
  .hud-bk.tr {
    top: 8px;
    right: 8px;
    border-top-width: 2px;
    border-right-width: 2px;
    border-top-right-radius: 6px;
  }
  .hud-bk.bl {
    bottom: 8px;
    left: 8px;
    border-bottom-width: 2px;
    border-left-width: 2px;
    border-bottom-left-radius: 6px;
  }
  .hud-bk.br {
    bottom: 8px;
    right: 8px;
    border-bottom-width: 2px;
    border-right-width: 2px;
    border-bottom-right-radius: 6px;
  }
  /* periodic scan sweep down the panel */
  .hud-scan {
    position: absolute;
    left: 0;
    right: 0;
    top: 0;
    z-index: 2;
    height: 38%;
    pointer-events: none;
    background: linear-gradient(
      to bottom,
      transparent,
      color-mix(in srgb, var(--info) 15%, transparent)
    );
    border-bottom: 1px solid color-mix(in srgb, var(--info) 45%, transparent);
    animation: hud-sweep 4.5s linear infinite;
  }
  @keyframes hud-sweep {
    0% {
      transform: translateY(-100%);
      opacity: 0;
    }
    12% {
      opacity: 1;
    }
    88% {
      opacity: 1;
    }
    100% {
      transform: translateY(290%);
      opacity: 0;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .dash-overlay,
    .dash-modal {
      animation: none;
    }
    .hud-scan {
      display: none;
    }
  }

  /* Broadcast banner — a loud, always-visible reminder while the powerful
     "type once, hit every agent" mode is armed. Click anywhere on it to disarm. */
  .bcast-banner {
    position: fixed;
    top: 10px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 9999;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 16px;
    font-size: 0.82rem;
    font-weight: 700;
    letter-spacing: 0.02em;
    color: var(--bg);
    background: var(--info);
    border: none;
    border-radius: 999px;
    cursor: pointer;
    animation: bcast-banner-pulse 1.4s ease-in-out infinite;
  }
  .bcast-banner .bolt {
    font-size: 1rem;
  }
  .bcast-banner .hint {
    font-weight: 600;
    opacity: 0.7;
    padding-left: 8px;
    border-left: 1px solid color-mix(in srgb, var(--bg) 35%, transparent);
  }
  @keyframes bcast-banner-pulse {
    0%,
    100% {
      box-shadow:
        0 0 0 1px color-mix(in srgb, var(--info) 50%, transparent),
        0 6px 18px color-mix(in srgb, var(--info) 35%, transparent);
    }
    50% {
      box-shadow:
        0 0 0 1px var(--info),
        0 8px 30px color-mix(in srgb, var(--info) 65%, transparent);
    }
  }
</style>
