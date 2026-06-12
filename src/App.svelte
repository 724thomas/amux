<script lang="ts">
  // M1: a single full-window terminal. M2 replaces this with the
  // sidebar + workspace layout tree.
  import { onMount } from "svelte";
  import Terminal from "./lib/Terminal.svelte";
  import { createPane, type PaneId } from "./lib/ipc";

  let pane = $state<PaneId | null>(null);
  let error = $state<string | null>(null);

  onMount(async () => {
    try {
      pane = await createPane(80, 24);
    } catch (e) {
      error = String(e);
    }
  });
</script>

<main class="app">
  {#if pane}
    <Terminal {pane} />
  {:else if error}
    <p class="error">터미널 생성 실패: {error}</p>
  {/if}
</main>

<style>
  .app {
    width: 100vw;
    height: 100vh;
    background: #16161e;
  }
  .error {
    padding: 1rem;
    color: #f7768e;
    font-family: monospace;
  }
</style>
