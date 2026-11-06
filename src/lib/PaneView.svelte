<script lang="ts">
  // One terminal pane: click-to-focus, focus ring, hover toolbar,
  // pane actions in the terminal's context menu, drag-rearrange.
  import Terminal from "./Terminal.svelte";
  import ChatView from "./chat/ChatView.svelte";
  import TermFiles from "./chat/TermFiles.svelte";
  import SubagentColumn from "./chat/SubagentColumn.svelte";
  import { adoptChat, chatOf, chatOpen, openChat, paneNotes, toggleChat } from "./chat/chat.svelte";
  import { setTermFiles, termFiles } from "./chat/termFiles.svelte";
  import { dockBack, popout } from "./chat/popout.svelte";
  import {
    closePane,
    focusPane,
    movePane,
    setPaneDone,
    splitPane,
    type PaneId,
    type SplitAxis,
  } from "./ipc";
  import { activeTab, paneInfo, rings, broadcast } from "./state.svelte";

  let { pane, focused }: { pane: PaneId; focused: boolean } = $props();

  const ringing = $derived(rings.active[pane] === true);
  const pending = $derived(paneInfo(pane)?.notification != null);
  // Broadcast: this pane is "armed" when broadcast mode is on and it sits in
  // the tab on screen — a broadcast never reaches past the visible tab.
  const broadcasting = $derived(broadcast.on && paneInfo(pane)?.tab === activeTab()?.id);

  // Done-Shockwave: when a pane finishes unseen (status → processed), fire a
  // one-shot radial burst to pull the eye to the agent that just shipped.
  // Bumping `burst` re-keys the effect element below, restarting the animation.
  const status = $derived(paneInfo(pane)?.status);
  // `done` is a pinned status, so it doubles as the toggle's own on/off state.
  const isDone = $derived(status === "done");
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

  // Structured Claude chat over this pane's terminal (see chat/).
  const chatOn = $derived(chatOpen(pane));
  // The engine says this pane shows a chat that this page knows nothing
  // about: the page was reloaded. Re-attach to (or clean up) that chat.
  $effect(() => {
    if (paneInfo(pane)?.chat && !chatOf(pane)) void adoptChat(pane);
  });

  // The files panel beside the terminal (CLI mode; the chat has its own).
  const filesOn = $derived(termFiles.open[pane] === true);
  /** The panel is in the pop-out window (then not shown here). */
  const popped = $derived(popout.panes.includes(pane));
  /** How handing the conversation between terminal and chat went. */
  const note = $derived(paneNotes.byPane[pane]);
  /** Width of the main part (the whole pane, or 3 of 3:2 beside subagents). */
  let rowWidth = $state(0);
  /** Too narrow for terminal and panel side by side: the panel goes over it. */
  const filesOver = $derived(rowWidth > 0 && rowWidth < 640);
  /** 📂 and its palette entry: show or hide the panel beside the terminal.
   *  While it is in the pop-out window, bring it back here instead (turning
   *  the flag off then would leave ↩ in the pop-out with nothing to show). */
  function toggleFiles() {
    if (popped) {
      setTermFiles(pane, true);
      dockBack(pane);
    } else setTermFiles(pane, !filesOn);
  }

  const extraActions = [
    { label: "Claude 채팅으로 열기 (Ctrl+Shift+M)", run: () => void openChat(pane) },
    { label: "바뀐 파일 보기 (파일 패널)", run: toggleFiles },
    { label: "오른쪽으로 분할", run: () => void splitPane(pane, "horizontal") },
    { label: "아래로 분할", run: () => void splitPane(pane, "vertical") },
    // Closing the last pane in a tab closes the tab (and an empty workspace
    // with it) — the cascade lives in the engine's close_pane.
    { label: "이 터미널 닫기", run: () => void closePane(pane) },
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
  <!-- The main part (the terminal with its files panel, or the chat over it)
       and, while the Claude in it has subagents, their column on the right
       (3:2). Both are always in the tree, so the column coming and going only
       resizes the main part (one terminal resize), never rebuilds it. -->
  <div class="pane-row">
    <div class="main-part" bind:clientWidth={rowWidth}>
    <div class="term-wrap">
      <!-- While the chat covers the terminal, `inert` keeps every focus path of
           the terminal (click, shortcuts, the waiting-status hand-off) from
           pulling the keyboard into a screen nobody can see. -->
      <div class="term-layer" inert={chatOn}>
        <Terminal {pane} {focused} {extraActions} />
      </div>
      <!-- Over the terminal part only, not over the panel's header. -->
      {#if !chatOn}
        {@render paneTools(false)}
      {/if}
    </div>
    {#if filesOn && !chatOn && !popped}
      <TermFiles {pane} overlay={filesOver} onclose={() => setTermFiles(pane, false)} />
    {/if}
    {#if chatOn}
      <ChatView {pane} {focused} tools={paneTools} />
    {/if}
    </div>
    <SubagentColumn {pane} chat={chatOn} />
  </div>
  {#if note}
    <div class="pane-note" class:error={note.tone === "error"} role="status">{note.text}</div>
  {/if}
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
</section>

<!-- Pane actions. Over a terminal they float at the top on hover; over a chat
     they sit at the end of the chat's header instead (a floating bar would
     cover the header the whole time the mouse is in the chat). -->
{#snippet paneTools(inline: boolean)}
  <div class="toolbar" class:inline>
    <button
      class="done-toggle"
      class:on={isDone}
      role="switch"
      aria-checked={isDone}
      title={isDone
        ? "DONE 고정 해제 — 상태 자동 갱신 재개"
        : "DONE으로 고정 — 작업 완료, 검토 중"}
      onmousedown={(e) => e.preventDefault()}
      onclick={() => void setPaneDone(pane, !isDone)}
    >
      ✓
    </button>
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
      class="chat-toggle"
      class:on={chatOn}
      title={chatOn
        ? "채팅 닫고 터미널로 (Ctrl+Shift+M)"
        : "Claude 채팅으로 열기 (Ctrl+Shift+M)"}
      onmousedown={(e) => e.preventDefault()}
      onclick={() => toggleChat(pane)}
    >
      💬
    </button>
    {#if !inline}
      <button
        class="files-toggle"
        class:on={filesOn || popped}
        title={popped
          ? "파일 창이 팝업 창에 있습니다 (누르면 다시 여기로)"
          : filesOn
            ? "파일 패널 닫기"
            : "바뀐 파일을 옆에서 보기 (Claude 가 고친 파일이 열리고, 바뀐 줄이 입력되듯 재생됩니다)"}
        onmousedown={(e) => e.preventDefault()}
        onclick={toggleFiles}
      >
        {popped ? "⧉" : "📂"}
      </button>
    {/if}
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
{/snippet}

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
  .pane-row {
    position: absolute;
    inset: 0;
    display: flex;
    /* A files panel lying over a narrow terminal starts below the pane's
       floating toolbar, which would otherwise cover the panel's header. */
    --files-top: 34px;
  }
  .pane-note {
    position: absolute;
    top: 40px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 6;
    max-width: min(80%, 560px);
    padding: 6px 12px;
    border-radius: 6px;
    border: 1px solid var(--border-2);
    background: var(--surface-2);
    color: var(--text);
    font-family: system-ui, sans-serif;
    font-size: 13px;
    line-height: 1.45;
    box-shadow: 0 4px 16px rgb(0 0 0 / 0.35);
    pointer-events: none;
  }
  .pane-note.error {
    border-color: color-mix(in srgb, var(--red) 60%, var(--border-2));
  }
  .main-part {
    position: relative;
    flex: 1 1 0;
    min-width: 0;
    display: flex;
  }
  .term-wrap {
    position: relative;
    flex: 1 1 auto;
    min-width: 0;
  }
  .term-layer {
    position: absolute;
    inset: 0;
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
  /* 시스템의 "애니메이션 사용" 설정(prefers-reduced-motion)에 반응하던 규칙을
     일부러 제거했다 — amux는 그 설정과 무관하게 동작한다. 자세한 경위는
     src/lib/Terminal.svelte의 .wave-lab 주석 참고. 되돌리지 말 것. */
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
  .toolbar.inline {
    position: static;
    display: flex;
    transform: none;
    padding: 0;
    background: none;
    border: none;
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
  /* The only toolbar button with a latched state: filled while DONE is pinned,
     so the toggle reads as on/off without opening the sidebar. */
  .toolbar button.done-toggle.on {
    background: var(--done);
    color: var(--bg);
  }
  .toolbar button.done-toggle.on:hover {
    background: color-mix(in srgb, var(--done) 78%, var(--bg));
  }
  .toolbar button.files-toggle.on,
  .toolbar button.chat-toggle.on {
    background: var(--accent);
    color: var(--bg);
  }
  .toolbar button.close:hover {
    background: var(--red);
    color: var(--bg);
  }
</style>
