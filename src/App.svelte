<script lang="ts">
  import { onMount } from "svelte";
  import Sidebar from "./lib/Sidebar.svelte";
  import SplitNode from "./lib/SplitNode.svelte";
  import { app, initState } from "./lib/state.svelte";
  import { handleKey } from "./lib/keymap";

  const snapshot = $derived(app.snapshot);

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
  <Sidebar />
  <main class="main">
    <!-- Every workspace stays mounted so its terminals keep their xterm
         buffers; only the active one is displayed. -->
    {#each snapshot?.workspaces ?? [] as ws (ws.id)}
      <div class="workspace" class:hidden={ws.id !== snapshot?.active_workspace}>
        <SplitNode node={ws.layout} workspace={ws.id} activePane={ws.active_pane} />
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
