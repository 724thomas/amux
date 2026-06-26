<script lang="ts">
  import { onMount } from "svelte";
  import Sidebar from "./lib/Sidebar.svelte";
  import SplitNode from "./lib/SplitNode.svelte";
  import { app, initState, broadcast, activeWorkspacePaneCount } from "./lib/state.svelte";
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
