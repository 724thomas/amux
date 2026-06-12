<script lang="ts">
  // Recursive renderer for the layout tree. Splits are addressed by `path`
  // (false = first child, true = second), matching the engine's layout.rs.
  import SplitNode from "./SplitNode.svelte";
  import PaneView from "./PaneView.svelte";
  import { setRatio, type LayoutNode, type PaneId, type WorkspaceId } from "./ipc";

  let {
    node,
    workspace,
    activePane,
    visible = true,
    path = [],
  }: {
    node: LayoutNode;
    workspace: WorkspaceId;
    activePane: PaneId | null;
    /// Whether this workspace is the one on screen. Hidden workspaces must
    /// not mark their active pane as focused — switching back would then
    /// never re-trigger keyboard focus (the prop never changes).
    visible?: boolean;
    path?: boolean[];
  } = $props();

  let container = $state<HTMLDivElement>()!;
  let dragging = $state(false);
  let dragRatio = $state(0.5);

  // While dragging, render the local ratio for zero-lag feedback;
  // otherwise mirror the engine.
  const ratio = $derived(
    dragging ? dragRatio : node.type === "split" ? node.ratio : 0.5,
  );

  let rafPending = false;

  function ratioFromPointer(e: PointerEvent): number {
    if (node.type !== "split") return 0.5;
    const rect = container.getBoundingClientRect();
    const value =
      node.axis === "horizontal"
        ? (e.clientX - rect.left) / rect.width
        : (e.clientY - rect.top) / rect.height;
    return Math.min(0.95, Math.max(0.05, value));
  }

  function startDrag(e: PointerEvent) {
    if (node.type !== "split") return;
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
    dragging = true;
    dragRatio = ratioFromPointer(e);
  }

  function onDrag(e: PointerEvent) {
    if (!dragging) return;
    dragRatio = ratioFromPointer(e);
    if (!rafPending) {
      rafPending = true;
      requestAnimationFrame(() => {
        rafPending = false;
        void setRatio(workspace, path, dragRatio);
      });
    }
  }

  function endDrag() {
    if (!dragging) return;
    dragging = false;
    void setRatio(workspace, path, dragRatio);
  }
</script>

{#if node.type === "leaf"}
  <PaneView pane={node.pane} focused={visible && node.pane === activePane} />
{:else}
  <div class="split {node.axis}" bind:this={container}>
    <div class="cell" style="flex-grow: {ratio}">
      <SplitNode node={node.first} {workspace} {activePane} {visible} path={[...path, false]} />
    </div>
    <div
      class="divider {node.axis}"
      role="separator"
      aria-orientation={node.axis === "horizontal" ? "vertical" : "horizontal"}
      onpointerdown={startDrag}
      onpointermove={onDrag}
      onpointerup={endDrag}
      onpointercancel={endDrag}
      ondblclick={() => void setRatio(workspace, path, 0.5)}
    ></div>
    <div class="cell" style="flex-grow: {1 - ratio}">
      <SplitNode node={node.second} {workspace} {activePane} {visible} path={[...path, true]} />
    </div>
  </div>
{/if}

<style>
  .split {
    display: flex;
    width: 100%;
    height: 100%;
    min-width: 0;
    min-height: 0;
  }
  .split.horizontal {
    flex-direction: row;
  }
  .split.vertical {
    flex-direction: column;
  }
  .cell {
    flex-basis: 0;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
  }
  .divider {
    flex: 0 0 4px;
    background: var(--border);
    z-index: 5;
    touch-action: none;
  }
  .divider:hover,
  .divider:active {
    background: var(--accent);
  }
  .divider.horizontal {
    cursor: col-resize;
  }
  .divider.vertical {
    cursor: row-resize;
  }
</style>
