<script lang="ts">
  // One terminal pane: click-to-focus, focus ring, hover toolbar,
  // pane actions in the terminal's context menu, drag-rearrange.
  import Terminal from "./Terminal.svelte";
  import { closePane, focusPane, movePane, splitPane, type PaneId, type SplitAxis } from "./ipc";
  import { app, paneInfo, rings, broadcast } from "./state.svelte";

  let { pane, focused }: { pane: PaneId; focused: boolean } = $props();

  const ringing = $derived(rings.active[pane] === true);
  const pending = $derived(paneInfo(pane)?.notification != null);
  // Broadcast: this pane is "armed" when broadcast mode is on and it lives in
  // the active workspace (the only panes a broadcast actually reaches).
  const broadcasting = $derived(
    broadcast.on && paneInfo(pane)?.workspace === app.snapshot?.active_workspace,
  );

  // Done-Shockwave: when a pane finishes unseen (status → processed), fire a
  // one-shot radial burst to pull the eye to the agent that just shipped.
  // Bumping `burst` re-keys the effect element below, restarting the animation.
  const status = $derived(paneInfo(pane)?.status);
  let burst = $state(0);
  let prevStatus: string | undefined;
  $effect(() => {
    const s = status;
    if (s === "processed" && prevStatus !== undefined && prevStatus !== "processed") {
      burst++;
    }
    prevStatus = s;
  });

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
    const source = e.dataTransfer?.getData("amux/pane");
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
  class:broadcasting
  role="group"
  onpointerdowncapture={() => {
    if (!focused) void focusPane(pane);
  }}
  ondragover={(e) => {
    if (e.dataTransfer?.types.includes("amux/pane")) {
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
  {#key burst}
    {#if burst > 0}
      <div class="fx" aria-hidden="true">
        <span class="ring"></span>
        <span class="flash"></span>
        {#each [0, 1, 2, 3, 4, 5, 6, 7] as i (i)}
          <span class="spark" style="--a:{i * 45}deg"></span>
        {/each}
      </div>
    {/if}
  {/key}
  <div class="toolbar">
    <button
      class="drag-handle"
      title="드래그해서 위치 이동"
      draggable="true"
      ondragstart={(e) => {
        e.dataTransfer?.setData("amux/pane", pane);
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

  /* Broadcast mode: every receiving pane wears a pulsing electric frame so it's
     unmistakable that input is being mirrored here. The outline paints over the
     terminal (same trick the focus ring uses); ::after adds an inner glow.
     Declared after .pane.focused so a focused + broadcasting pane shows this. */
  .pane.broadcasting {
    outline: 2px solid var(--info);
    outline-offset: -2px;
    animation: bcast-frame 1.4s ease-in-out infinite;
  }
  @keyframes bcast-frame {
    0%,
    100% {
      outline-color: color-mix(in srgb, var(--info) 45%, transparent);
    }
    50% {
      outline-color: var(--info);
    }
  }
  .pane.broadcasting::after {
    content: "";
    position: absolute;
    inset: 0;
    z-index: 5;
    pointer-events: none;
    box-shadow: inset 0 0 18px -3px color-mix(in srgb, var(--info) 55%, transparent);
    animation: bcast-glow 1.4s ease-in-out infinite;
  }
  @keyframes bcast-glow {
    0%,
    100% {
      opacity: 0.5;
    }
    50% {
      opacity: 1;
    }
  }

  /* Done-Shockwave — a one-shot radial burst when an agent finishes unseen.
     Pure transform/opacity (GPU-composited), pointer-events:none, above the
     terminal but below the toolbar. Green to match the "processed" chip. */
  .fx {
    position: absolute;
    left: 50%;
    top: 50%;
    width: 0;
    height: 0;
    z-index: 6;
    pointer-events: none;
  }
  .fx .ring {
    position: absolute;
    left: -22px;
    top: -22px;
    width: 44px;
    height: 44px;
    border-radius: 50%;
    border: 2.5px solid var(--green);
    transform: scale(0.2);
    opacity: 0.9;
    animation: sw-ring 0.85s cubic-bezier(0.2, 0.7, 0.3, 1) forwards;
  }
  @keyframes sw-ring {
    0% {
      transform: scale(0.2);
      opacity: 0.9;
    }
    100% {
      transform: scale(12);
      opacity: 0;
      border-width: 0.5px;
    }
  }
  .fx .flash {
    position: absolute;
    left: -32px;
    top: -32px;
    width: 64px;
    height: 64px;
    border-radius: 50%;
    background: radial-gradient(
      circle,
      color-mix(in srgb, var(--green) 55%, transparent),
      transparent 70%
    );
    transform: scale(0.3);
    opacity: 0.85;
    animation: sw-flash 0.5s ease-out forwards;
  }
  @keyframes sw-flash {
    0% {
      transform: scale(0.3);
      opacity: 0.85;
    }
    100% {
      transform: scale(2.4);
      opacity: 0;
    }
  }
  .fx .spark {
    position: absolute;
    left: 0;
    top: 0;
    width: 4px;
    height: 4px;
    margin: -2px 0 0 -2px;
    border-radius: 50%;
    background: var(--green);
    animation: sw-spark 0.7s ease-out forwards;
  }
  @keyframes sw-spark {
    0% {
      transform: rotate(var(--a)) translateY(-6px) scale(1);
      opacity: 1;
    }
    100% {
      transform: rotate(var(--a)) translateY(-72px) scale(0.3);
      opacity: 0;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .fx {
      display: none;
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
    left: 50%;
    transform: translateX(-50%);
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
