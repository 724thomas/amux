<script lang="ts">
  // One terminal pane: click-to-focus, focus ring, hover toolbar,
  // and pane actions merged into the terminal's context menu.
  import Terminal from "./Terminal.svelte";
  import { closePane, focusPane, splitPane, type PaneId } from "./ipc";
  import { paneInfo, rings } from "./state.svelte";

  let { pane, focused }: { pane: PaneId; focused: boolean } = $props();

  const ringing = $derived(rings.active[pane] === true);
  const pending = $derived(paneInfo(pane)?.notification != null);

  const extraActions = [
    { label: "오른쪽으로 분할", run: () => void splitPane(pane, "horizontal") },
    { label: "아래로 분할", run: () => void splitPane(pane, "vertical") },
    { label: "Pane 닫기", run: () => void closePane(pane) },
  ];
</script>

<section
  class="pane"
  class:focused
  class:ringing
  onpointerdowncapture={() => {
    if (!focused) void focusPane(pane);
  }}
>
  <Terminal {pane} {focused} {extraActions} />
  {#if pending}
    <span class="pending-dot" title="알림 대기 중"></span>
  {/if}
  <div class="toolbar">
    <button title="오른쪽으로 분할 (Ctrl+Shift+D)" onclick={() => void splitPane(pane, "horizontal")}>
      ◫
    </button>
    <button title="아래로 분할 (Ctrl+Shift+S)" onclick={() => void splitPane(pane, "vertical")}>
      ⬓
    </button>
    <button title="Pane 닫기 (Ctrl+Shift+W)" class="close" onclick={() => void closePane(pane)}>
      ✕
    </button>
  </div>
</section>

<style>
  .pane {
    position: relative;
    width: 100%;
    height: 100%;
    min-width: 0;
    min-height: 0;
    outline: 1px solid #2a2e42;
    outline-offset: -1px;
  }
  .pane.focused {
    outline: 1px solid #7aa2f7;
    z-index: 1;
  }
  .pane.ringing {
    animation: ring-pulse 0.75s ease-in-out 4;
  }
  @keyframes ring-pulse {
    0%,
    100% {
      outline: 2px solid transparent;
      outline-offset: -2px;
    }
    50% {
      outline: 2px solid #7dcfff;
      outline-offset: -2px;
    }
  }
  .pending-dot {
    position: absolute;
    top: 6px;
    left: 8px;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: #7dcfff;
    z-index: 10;
  }
  .toolbar {
    position: absolute;
    top: 4px;
    right: 8px;
    display: none;
    gap: 2px;
    padding: 2px;
    background: rgba(31, 35, 53, 0.9);
    border: 1px solid #3b4261;
    border-radius: 6px;
    z-index: 10;
  }
  .pane:hover .toolbar {
    display: flex;
  }
  .toolbar button {
    width: 24px;
    height: 22px;
    color: #c0caf5;
    background: none;
    border: none;
    border-radius: 4px;
    cursor: pointer;
    font-size: 0.8rem;
    line-height: 1;
  }
  .toolbar button:hover {
    background: #3b4261;
  }
  .toolbar button.close:hover {
    background: #f7768e;
    color: #16161e;
  }
</style>
