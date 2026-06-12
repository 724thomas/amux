<script lang="ts">
  import { onMount } from "svelte";
  import Sidebar from "./lib/Sidebar.svelte";
  import SplitNode from "./lib/SplitNode.svelte";
  import { app, initState } from "./lib/state.svelte";
  import { handleKey } from "./lib/keymap";
  import { setSidebarWidth, settings } from "./lib/settings.svelte";

  const snapshot = $derived(app.snapshot);

  let draggingSidebar = $state(false);

  onMount(() => {
    void initState();
  });
</script>

<!-- Shortcuts also work when focus is outside any terminal. -->
<svelte:window
  onkeydown={(e) => {
    if (handleKey(e)) e.preventDefault();
  }}
/>

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
    background: #16161e;
  }
  .sidebar-wrap {
    flex-shrink: 0;
    min-width: 0;
    display: flex;
  }
  .sidebar-resizer {
    flex: 0 0 4px;
    cursor: col-resize;
    background: #2a2e42;
    touch-action: none;
  }
  .sidebar-resizer:hover,
  .sidebar-resizer.dragging {
    background: #7aa2f7;
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
</style>
