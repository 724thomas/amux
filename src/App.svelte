<script lang="ts">
  import { onMount, tick } from "svelte";
  import Sidebar from "./lib/Sidebar.svelte";
  import SplitNode from "./lib/SplitNode.svelte";
  import Palette from "./lib/Palette.svelte";
  import Dashboard from "./lib/Dashboard.svelte";
  import {
    app,
    initState,
    broadcast,
    palette,
    dashboard,
    activeTabPaneCount,
    focusTerm,
    tabCreate,
    tabHasBadge,
    tabStatus,
  } from "./lib/state.svelte";
  import {
    closeTab,
    focusTab,
    moveTab,
    newTab,
    renameTab,
    type TabId,
    type WorkspaceId,
  } from "./lib/ipc";
  import { handleKey } from "./lib/keymap";
  import { setSidebarWidth, settings } from "./lib/settings.svelte";
  import { themeById } from "./lib/themes";

  const snapshot = $derived(app.snapshot);
  const bcastCount = $derived(activeTabPaneCount());

  let draggingSidebar = $state(false);

  // Tab-bar interactions. Renaming is inline (double-click a tab); dragging a
  // tab onto another reorders it, mirroring the sidebar's workspace drag.
  let renamingTab = $state<TabId | null>(null);
  let renameValue = $state("");
  let draggedTab = $state<TabId | null>(null);

  function startRenameTab(tab: TabId, name: string) {
    renamingTab = tab;
    renameValue = name;
    void tick().then(() => document.querySelector<HTMLInputElement>(".tab-rename")?.select());
  }

  function commitRenameTab() {
    if (renamingTab && renameValue.trim()) void renameTab(renamingTab, renameValue.trim());
    renamingTab = null;
  }

  // New-tab title prompt. Opened by the tab bar "+", Ctrl+T or the palette —
  // all three only set `tabCreate.workspace`, and the input below is the single
  // place a tab is actually born. A blank name falls through to the engine's
  // auto-name (`탭 N`), so Ctrl+T then Enter is still a one-beat "just give me
  // a tab".
  let newTabName = $state("");
  let newTabInput = $state<HTMLInputElement | null>(null);
  // A plain `autofocus` attribute doesn't fire on a dynamically-mounted input
  // in this webview (the Palette and the new-workspace prompt hit the same
  // thing), so focus it explicitly once the prompt opens.
  $effect(() => {
    if (tabCreate.workspace) newTabInput?.focus();
  });

  function startCreateTab(workspace: WorkspaceId) {
    newTabName = "";
    tabCreate.workspace = workspace;
  }

  function commitCreateTab(workspace: WorkspaceId) {
    const name = newTabName.trim();
    tabCreate.workspace = null;
    newTabName = "";
    void newTab(workspace, name || undefined);
  }

  function cancelCreateTab() {
    tabCreate.workspace = null;
    newTabName = "";
  }

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
    BROADCAST — 입력이 이 탭의 {bcastCount}개 pane에 동시 전송됩니다
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
    <!-- Every workspace AND every tab stays mounted so its terminals keep
         their xterm buffers and their agents keep running; only the active
         one is displayed. Never unmount — `display: none` only. -->
    {#each snapshot?.workspaces ?? [] as ws (ws.id)}
      {@const wsVisible = ws.id === snapshot?.active_workspace}
      <div class="workspace" class:hidden={!wsVisible}>
        <div class="tabbar">
          {#each ws.tabs as tab, index (tab.id)}
            {@const on = tab.id === ws.active_tab}
            <div
              class="tab"
              class:on
              data-status={tabStatus(tab) ?? "idle"}
              role="tab"
              tabindex="-1"
              aria-selected={on}
              draggable={renamingTab !== tab.id}
              onclick={() => {
                void focusTab(tab.id);
                focusTerm(tab.active_pane);
              }}
              ondblclick={() => startRenameTab(tab.id, tab.name)}
              onauxclick={(e) => {
                if (e.button === 1) void closeTab(tab.id); // middle-click closes
              }}
              onkeydown={() => {}}
              ondragstart={() => (draggedTab = tab.id)}
              ondragover={(e) => e.preventDefault()}
              ondrop={(e) => {
                e.preventDefault();
                if (draggedTab) void moveTab(draggedTab, index);
                draggedTab = null;
              }}
            >
              <span class="dot"></span>
              {#if renamingTab === tab.id}
                <input
                  class="tab-rename"
                  bind:value={renameValue}
                  onblur={commitRenameTab}
                  onclick={(e) => e.stopPropagation()}
                  ondblclick={(e) => e.stopPropagation()}
                  onkeydown={(e) => {
                    if (e.key === "Enter") commitRenameTab();
                    if (e.key === "Escape") renamingTab = null;
                    e.stopPropagation();
                  }}
                />
              {:else}
                <span class="tab-name">{tab.name}</span>
                {#if tabHasBadge(tab)}<span class="tab-badge"></span>{/if}
                <button
                  class="tab-close"
                  title="탭 닫기 (Ctrl+W)"
                  onclick={(e) => {
                    e.stopPropagation();
                    void closeTab(tab.id);
                  }}>×</button
                >
              {/if}
            </div>
          {/each}
          {#if tabCreate.workspace === ws.id}
            <input
              class="tab-new-input"
              bind:this={newTabInput}
              placeholder="새 탭 이름 (Enter 생성 · Esc 취소)"
              bind:value={newTabName}
              onblur={cancelCreateTab}
              onclick={(e) => e.stopPropagation()}
              onkeydown={(e) => {
                if (e.key === "Enter") commitCreateTab(ws.id);
                else if (e.key === "Escape") cancelCreateTab();
                e.stopPropagation();
              }}
            />
          {:else}
            <button class="tab-add" title="새 탭 (Ctrl+T)" onclick={() => startCreateTab(ws.id)}>
              +
            </button>
          {/if}
        </div>
        <div class="tab-body">
          {#each ws.tabs as tab (tab.id)}
            <div class="tab-panel" class:hidden={tab.id !== ws.active_tab}>
              <SplitNode
                node={tab.layout}
                workspace={ws.id}
                tab={tab.id}
                activePane={tab.active_pane}
                visible={wsVisible && tab.id === ws.active_tab}
              />
            </div>
          {/each}
        </div>
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
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .workspace.hidden {
    display: none;
  }

  /* Tab bar — one row per workspace, above its terminals. A tab is the named
     unit here; the split tree lives inside it. */
  .tabbar {
    flex: 0 0 auto;
    display: flex;
    align-items: stretch;
    gap: 2px;
    padding: 4px 6px 0;
    background: var(--bg);
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
    scrollbar-width: thin;
  }
  /* Tabs share the bar's full width instead of hugging their text: `flex: 1 1 0`
     gives every tab an equal slice of whatever is left over. `min-width` is the
     floor — once enough tabs exist that they'd go below it, they stop shrinking
     and the bar scrolls instead. */
  .tab {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: 1 1 0;
    min-width: 120px;
    padding: 6px 10px 6px 12px;
    font-size: 0.78rem;
    color: var(--muted);
    background: color-mix(in srgb, var(--text) 6%, transparent);
    border: 1px solid transparent;
    border-bottom: none;
    border-radius: 7px 7px 0 0;
    cursor: pointer;
    user-select: none;
    white-space: nowrap;
  }
  .tab:hover {
    background: color-mix(in srgb, var(--text) 11%, transparent);
  }
  .tab.on {
    color: var(--text);
    background: var(--surface-3);
    border-color: var(--border);
    box-shadow: inset 0 2px 0 var(--accent);
  }
  /* The name takes the slack between the status dot and the close button, so a
     wide tab reads centred rather than with the text stuck to the left edge. */
  .tab-name {
    flex: 1;
    min-width: 0;
    text-align: center;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  /* Status colour = the most attention-hungry pane inside the tab. */
  .tab .dot {
    width: 7px;
    height: 7px;
    flex-shrink: 0;
    border-radius: 50%;
    background: var(--accent);
  }
  .tab[data-status="processing"] .dot {
    background: var(--red);
  }
  .tab[data-status="processed"] .dot {
    background: var(--green);
  }
  .tab[data-status="waiting"] .dot {
    background: var(--yellow);
  }
  .tab[data-status="done"] .dot {
    background: var(--done);
  }
  .tab-badge {
    width: 6px;
    height: 6px;
    flex-shrink: 0;
    border-radius: 50%;
    background: var(--info);
  }
  .tab-close,
  .tab-add {
    flex-shrink: 0;
    padding: 0 4px;
    font-size: 0.95rem;
    line-height: 1;
    color: var(--muted);
    background: none;
    border: none;
    border-radius: 4px;
    cursor: pointer;
  }
  .tab-close:hover,
  .tab-add:hover {
    color: var(--text);
    background: color-mix(in srgb, var(--text) 16%, transparent);
  }
  .tab-add {
    align-self: center;
    padding: 2px 8px;
    font-size: 1rem;
  }
  .tab-rename {
    flex: 1;
    min-width: 0;
    padding: 0;
    font: inherit;
    text-align: center;
    color: var(--text);
    background: transparent;
    border: none;
    border-bottom: 1px solid var(--accent);
    outline: none;
  }
  /* Sits where the "+" was, sized like a tab so the bar doesn't jump. */
  .tab-new-input {
    flex: 1 1 0;
    min-width: 120px;
    align-self: center;
    padding: 6px 10px;
    font: inherit;
    font-size: 0.78rem;
    color: var(--text);
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    border: 1px solid var(--accent);
    border-radius: 7px 7px 0 0;
    outline: none;
  }
  .tab-new-input::placeholder {
    color: var(--muted);
  }
  .tab-body {
    position: relative;
    flex: 1;
    min-height: 0;
  }
  .tab-panel {
    position: absolute;
    inset: 0;
  }
  .tab-panel.hidden {
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
  /* 시스템의 "애니메이션 사용" 설정(prefers-reduced-motion)에 반응하던 규칙을
     일부러 제거했다 — amux는 그 설정과 무관하게 동작한다. 자세한 경위는
     src/lib/Terminal.svelte의 .wave-lab 주석 참고. 되돌리지 말 것. */

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
