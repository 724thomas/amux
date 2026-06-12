<script lang="ts">
  // One terminal pane: click-to-focus, focus ring, hover toolbar,
  // pane actions in the terminal's context menu, drag-rearrange.
  import Terminal from "./Terminal.svelte";
  import { closePane, focusPane, movePane, splitPane, type PaneId, type SplitAxis } from "./ipc";
  import { paneInfo, rings } from "./state.svelte";

  let { pane, focused }: { pane: PaneId; focused: boolean } = $props();

  const ringing = $derived(rings.active[pane] === true);
  const pending = $derived(paneInfo(pane)?.notification != null);

  type DropZone = "left" | "right" | "top" | "bottom";
  let dropZone = $state<DropZone | null>(null);

  const extraActions = [
    { label: "오른쪽으로 분할", run: () => void splitPane(pane, "horizontal") },
    { label: "아래로 분할", run: () => void splitPane(pane, "vertical") },
    { label: "Pane 닫기", run: () => void closePane(pane) },
  ];

  function zoneOf(e: DragEvent, el: HTMLElement): DropZone {
    const rect = el.getBoundingClientRect();
    const x = (e.clientX - rect.left) / rect.width;
    const y = (e.clientY - rect.top) / rect.height;
    // Nearest edge wins.
    const distances: [DropZone, number][] = [
      ["left", x],
      ["right", 1 - x],
      ["top", y],
      ["bottom", 1 - y],
    ];
    distances.sort((a, b) => a[1] - b[1]);
    return distances[0][0];
  }

  function onDrop(e: DragEvent) {
    e.preventDefault();
    const source = e.dataTransfer?.getData("cmux/pane");
    const zone = dropZone;
    dropZone = null;
    if (!source || source === pane || !zone) return;
    const axis: SplitAxis = zone === "left" || zone === "right" ? "horizontal" : "vertical";
    const before = zone === "left" || zone === "top";
    void movePane(source, pane, axis, before);
  }
</script>

<section
  class="pane"
  class:focused
  class:ringing
  role="group"
  onpointerdowncapture={() => {
    if (!focused) void focusPane(pane);
  }}
  ondragover={(e) => {
    if (e.dataTransfer?.types.includes("cmux/pane")) {
      e.preventDefault();
      dropZone = zoneOf(e, e.currentTarget as HTMLElement);
    }
  }}
  ondragleave={() => (dropZone = null)}
  ondrop={onDrop}
>
  <Terminal {pane} {focused} {extraActions} />
  {#if pending}
    <span class="pending-dot" title="알림 대기 중"></span>
  {/if}
  {#if dropZone}
    <div class="drop-overlay {dropZone}"></div>
  {/if}
  <div class="toolbar">
    <button
      class="drag-handle"
      title="드래그해서 위치 이동"
      draggable="true"
      ondragstart={(e) => {
        e.dataTransfer?.setData("cmux/pane", pane);
        if (e.dataTransfer) e.dataTransfer.effectAllowed = "move";
      }}
    >
      ⠿
    </button>
    <button
      title="오른쪽으로 분할 (Ctrl+Shift+D)"
      onmousedown={(e) => e.preventDefault()}
      onclick={() => void splitPane(pane, "horizontal")}
    >
      ◫
    </button>
    <button
      title="아래로 분할 (Ctrl+Shift+S)"
      onmousedown={(e) => e.preventDefault()}
      onclick={() => void splitPane(pane, "vertical")}
    >
      ⬓
    </button>
    <button
      title="Pane 닫기 (Ctrl+Shift+W)"
      class="close"
      onmousedown={(e) => e.preventDefault()}
      onclick={() => void closePane(pane)}
    >
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
    outline: 1px solid var(--border);
    outline-offset: -1px;
  }
  .pane.focused {
    outline: 1px solid var(--accent);
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
      outline: 2px solid var(--info);
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
    background: var(--info);
    z-index: 10;
  }
  .drop-overlay {
    position: absolute;
    z-index: 15;
    background: color-mix(in srgb, var(--accent) 25%, transparent);
    border: 2px solid var(--accent);
    pointer-events: none;
  }
  .drop-overlay.left {
    inset: 0 50% 0 0;
  }
  .drop-overlay.right {
    inset: 0 0 0 50%;
  }
  .drop-overlay.top {
    inset: 0 0 50% 0;
  }
  .drop-overlay.bottom {
    inset: 50% 0 0 0;
  }
  .toolbar {
    position: absolute;
    top: 4px;
    right: 8px;
    display: none;
    gap: 2px;
    padding: 2px;
    background: color-mix(in srgb, var(--surface-2) 90%, transparent);
    border: 1px solid var(--border-2);
    border-radius: 6px;
    z-index: 10;
  }
  .pane:hover .toolbar {
    display: flex;
  }
  .toolbar button {
    width: 24px;
    height: 22px;
    color: var(--text);
    background: none;
    border: none;
    border-radius: 4px;
    cursor: pointer;
    font-size: 0.8rem;
    line-height: 1;
  }
  .toolbar button:hover {
    background: var(--border-2);
  }
  .toolbar .drag-handle {
    cursor: grab;
  }
  .toolbar button.close:hover {
    background: var(--red);
    color: var(--bg);
  }
</style>
