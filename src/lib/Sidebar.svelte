<script lang="ts">
  // Vertical list: workspaces with their terminals nested beneath
  // (워크스페이스 1 → 터미널 1, 터미널 2 ...). Mouse-first: click to switch,
  // drag to reorder workspaces, right-click to rename/close, port chips
  // open the browser, + creates a workspace. The bottom panel is the
  // "지금 봐야 할 에이전트" triage list (손길 필요한 pane 우선순위).
  import { openUrl } from "@tauri-apps/plugin-opener";
  import {
    closePane,
    closeWorkspace,
    createWorkspace,
    focusPane,
    focusWorkspace,
    moveWorkspace,
    renamePane,
    renameWorkspace,
    type LayoutNode,
    type PaneId,
    type WorkspaceId,
    type WorkspaceInfo,
  } from "./ipc";
  import { app, attentionItems, clock, focusTerm, paneInfo } from "./state.svelte";
  import { adjustFontSize, setNotifHeight, setTheme, settings } from "./settings.svelte";
  import { THEMES, themeById } from "./themes";

  const snapshot = $derived(app.snapshot);
  // 지금 봐야 할 에이전트 (🟡 waiting → 🟢 processed, 오래된 순) — 하단 패널.
  const attn = $derived(attentionItems());

  // Animated "processing" indicator: processing. → .. → ... → . (cycles).
  let dots = $state(1);
  $effect(() => {
    const t = setInterval(() => (dots = (dots % 3) + 1), 450);
    return () => clearInterval(t);
  });

  // Drag-resize the bottom panel; growing it eats into the workspace list.
  let panelDrag = $state<{ y: number; h: number } | null>(null);

  type MenuTarget =
    | { kind: "workspace"; id: WorkspaceId; name: string }
    | { kind: "pane"; id: PaneId; name: string };

  let menu = $state<{ x: number; y: number; target: MenuTarget } | null>(null);
  let themeMenuOpen = $state(false);
  let renaming = $state<{ kind: "workspace" | "pane"; id: string } | null>(null);
  let renameValue = $state("");
  let draggedId = $state<WorkspaceId | null>(null);

  function layoutPanes(node: LayoutNode): PaneId[] {
    return node.type === "leaf"
      ? [node.pane]
      : [...layoutPanes(node.first), ...layoutPanes(node.second)];
  }

  function wsPorts(ws: WorkspaceInfo): number[] {
    const ports = new Set<number>();
    for (const pane of snapshot?.panes ?? []) {
      if (pane.workspace !== ws.id) continue;
      for (const port of pane.meta.listening_ports) ports.add(port);
    }
    return [...ports].sort((a, b) => a - b);
  }

  function shortCwd(cwd: string | null): string {
    if (!cwd) return "";
    const parts = cwd.split("/").filter(Boolean);
    return parts.length > 2 ? "…/" + parts.slice(-2).join("/") : cwd;
  }

  function openMenu(e: MouseEvent, target: MenuTarget) {
    e.preventDefault();
    e.stopPropagation();
    menu = { x: e.clientX, y: e.clientY, target };
  }

  function startRename(target: MenuTarget) {
    menu = null;
    renaming = { kind: target.kind, id: target.id };
    renameValue = target.name;
  }

  function commitRename() {
    if (renaming && renameValue.trim()) {
      const name = renameValue.trim();
      if (renaming.kind === "workspace") void renameWorkspace(renaming.id, name);
      else void renamePane(renaming.id, name);
    }
    renaming = null;
  }

  function closeTarget(target: MenuTarget) {
    menu = null;
    if (target.kind === "workspace") void closeWorkspace(target.id);
    else void closePane(target.id);
  }

  // Live mm:ss since the status began, for the attention rows.
  function fmt(since: number): string {
    const s = Math.max(0, Math.floor((clock.now - since) / 1000));
    return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
  }
</script>

{#snippet renameInput()}
  <!-- svelte-ignore a11y_autofocus -->
  <input
    class="rename"
    autofocus
    bind:value={renameValue}
    onblur={commitRename}
    onkeydown={(e) => {
      if (e.key === "Enter") commitRename();
      if (e.key === "Escape") renaming = null;
      e.stopPropagation();
    }}
    onclick={(e) => e.stopPropagation()}
  />
{/snippet}

<svelte:window
  onclick={() => {
    menu = null;
    themeMenuOpen = false;
  }}
/>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<nav
  class="sidebar"
  onmousedown={(e) => {
    // Sidebar buttons must not steal the keyboard from the terminal.
    // Inputs (rename), selects (theme) and draggables keep defaults.
    const t = e.target as HTMLElement;
    if (t.closest("input, select") || t.closest('[draggable="true"]')) return;
    e.preventDefault();
  }}
>
  <ul class="workspaces">
    {#each snapshot?.workspaces ?? [] as ws, index (ws.id)}
      {@const ports = wsPorts(ws)}
      {@const isActiveWs = ws.id === snapshot?.active_workspace}
      <li>
        <button
          class="entry"
          class:active={isActiveWs}
          draggable={renaming?.id !== ws.id}
          onclick={() => {
            void focusWorkspace(ws.id);
            // Already-active workspace: no snapshot change will arrive, so
            // hand the keyboard to its terminal right now.
            focusTerm(ws.active_pane);
          }}
          oncontextmenu={(e) => openMenu(e, { kind: "workspace", id: ws.id, name: ws.name })}
          ondragstart={() => (draggedId = ws.id)}
          ondragover={(e) => e.preventDefault()}
          ondrop={(e) => {
            e.preventDefault();
            if (draggedId) void moveWorkspace(draggedId, index);
            draggedId = null;
          }}
        >
          {#if renaming?.kind === "workspace" && renaming.id === ws.id}
            {@render renameInput()}
          {:else}
            <span class="name">{ws.name}</span>
          {/if}
          {#if ports.length > 0}
            <span class="ports">
              {#each ports as port (port)}
                <span
                  class="port"
                  role="link"
                  tabindex="-1"
                  title="localhost:{port} 브라우저로 열기"
                  onclick={(e) => {
                    e.stopPropagation();
                    void openUrl(`http://localhost:${port}`);
                  }}
                  onkeydown={() => {}}
                >
                  :{port}
                </span>
              {/each}
            </span>
          {/if}
        </button>
        <ul class="panes">
          {#each layoutPanes(ws.layout) as paneId (paneId)}
            {@const pane = paneInfo(paneId)}
            <li>
              <button
                class="pane-entry"
                class:active={isActiveWs && paneId === ws.active_pane}
                onclick={() => {
                  void focusPane(paneId);
                  focusTerm(paneId);
                }}
                oncontextmenu={(e) =>
                  openMenu(e, { kind: "pane", id: paneId, name: pane?.name ?? "" })}
              >
                {#if renaming?.kind === "pane" && renaming.id === paneId}
                  {@render renameInput()}
                {:else}
                  <span class="pane-name">
                    {pane?.name ?? "터미널"}
                    {#if pane}
                      <span class="status {pane.status}">
                        {pane.status === "processing" ? "processing" + ".".repeat(dots) : pane.status}
                      </span>
                    {/if}
                    {#if pane?.notification}<span class="badge"></span>{/if}
                  </span>
                  <span class="pane-detail">
                    {#if pane?.meta.git_branch}<span class="branch">⎇ {pane.meta.git_branch}</span
                      >{/if}
                    <span class="cwd">{shortCwd(pane?.meta.cwd ?? null)}</span>
                  </span>
                {/if}
              </button>
            </li>
          {/each}
        </ul>
      </li>
    {/each}
  </ul>
  <button class="add" onclick={() => void createWorkspace()}>+ 새 워크스페이스</button>

  <!-- 하단 패널 높이 조절 핸들 (위로 드래그하면 패널이 커지고 워크스페이스 영역이 줄어듦) -->
  <div
    class="panel-resizer"
    class:dragging={panelDrag !== null}
    role="separator"
    aria-orientation="horizontal"
    onpointerdown={(e) => {
      panelDrag = { y: e.clientY, h: settings.notifHeight };
      (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    }}
    onpointermove={(e) => {
      if (panelDrag) setNotifHeight(panelDrag.h + (panelDrag.y - e.clientY));
    }}
    onpointerup={() => (panelDrag = null)}
    onpointercancel={() => (panelDrag = null)}
  ></div>

  <!-- 하단: 지금 봐야 할 에이전트 (손길 필요한 pane 우선순위 — 클릭 시 점프) -->
  <div class="attn-panel" style="height: {settings.notifHeight}px">
    <div class="attn-head">
      <span class="warn">⚠</span>
      <span>지금 봐야 할 에이전트</span>
      {#if attn.length > 0}<span class="attn-count">{attn.length}</span>{/if}
    </div>
    <ul class="attn-list">
      {#each attn as it (it.pane)}
        <li>
          <button
            class="attn-entry"
            onclick={() => {
              void focusPane(it.pane).catch(() => {});
              focusTerm(it.pane);
            }}
          >
            <span class="attn-dot {it.status}"></span>
            <span class="attn-nm">{it.name}</span>
            {#if it.workspace}<span class="attn-ws">{it.workspace}</span>{/if}
            <span class="attn-lbl">{it.status === "waiting" ? "입력 대기" : "완료·미확인"}</span>
            <span class="attn-time">{fmt(it.since)}</span>
          </button>
        </li>
      {:else}
        <li class="attn-empty">손길 필요한 에이전트 없음</li>
      {/each}
    </ul>
  </div>

  <div class="theme-control" title="색 테마">
    <span class="font-label">테마</span>
    <button
      class="theme-btn"
      onclick={(e) => {
        e.stopPropagation();
        themeMenuOpen = !themeMenuOpen;
      }}
    >
      <span class="swatch" style="background: {themeById(settings.theme).term.background}"></span>
      <span class="theme-name">{themeById(settings.theme).name}</span>
      <span class="caret">▴</span>
    </button>
    {#if themeMenuOpen}
      <div class="theme-menu">
        {#each THEMES as t (t.id)}
          <button
            class:selected={t.id === settings.theme}
            onclick={() => {
              setTheme(t.id);
              themeMenuOpen = false;
            }}
          >
            <span class="swatch" style="background: {t.term.background}"></span>
            {t.name}
          </button>
        {/each}
      </div>
    {/if}
  </div>

  <div class="font-control" title="글꼴 크기 (Ctrl+= / Ctrl+- / Ctrl+휠)">
    <span class="font-label">Aa</span>
    <button onclick={() => adjustFontSize(-1)}>−</button>
    <span class="font-size">{settings.fontSize}px</span>
    <button onclick={() => adjustFontSize(1)}>＋</button>
  </div>
</nav>

{#if menu}
  {@const target = menu.target}
  <div class="ctx-menu" style="left: {menu.x}px; top: {menu.y}px">
    <button onclick={() => startRename(target)}>이름 변경</button>
    <button onclick={() => closeTarget(target)}>
      {target.kind === "workspace" ? "워크스페이스 닫기" : "터미널 닫기"}
    </button>
  </div>
{/if}

<style>
  .sidebar {
    display: flex;
    flex-direction: column;
    width: 100%;
    background: var(--surface);
    overflow-y: auto;
  }
  ul {
    list-style: none;
  }
  .workspaces {
    flex: 1;
    overflow-y: auto;
    min-height: 0;
  }
  .panel-resizer {
    flex: 0 0 5px;
    cursor: row-resize;
    background: var(--border);
    touch-action: none;
  }
  .panel-resizer:hover,
  .panel-resizer.dragging {
    background: var(--accent);
  }

  /* ── 하단: 지금 봐야 할 에이전트 패널 ── */
  .attn-panel {
    display: flex;
    flex-direction: column;
    flex-shrink: 0;
    min-height: 0;
  }
  .attn-head {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px 4px;
    font-size: 0.72rem;
    font-weight: 700;
    color: var(--text-2);
  }
  .attn-head .warn {
    color: var(--yellow);
  }
  .attn-head .attn-count {
    margin-left: auto;
    min-width: 16px;
    padding: 0 6px;
    text-align: center;
    font-size: 0.68rem;
    border-radius: 9px;
    background: var(--yellow);
    color: var(--bg);
  }
  .attn-list {
    overflow-y: auto;
  }
  .attn-empty {
    padding: 4px 10px 8px;
    font-size: 0.72rem;
    color: var(--border-2);
  }
  .attn-entry {
    display: flex;
    align-items: center;
    gap: 7px;
    width: 100%;
    padding: 5px 10px;
    text-align: left;
    background: none;
    border: none;
    color: var(--text);
    cursor: pointer;
    font-size: 0.74rem;
  }
  .attn-entry:hover {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  .attn-dot {
    flex-shrink: 0;
    width: 7px;
    height: 7px;
    border-radius: 50%;
  }
  .attn-dot.waiting {
    background: var(--yellow);
    animation: attn-pulse 1.2s ease-in-out infinite;
  }
  .attn-dot.processed {
    background: var(--green);
  }
  @keyframes attn-pulse {
    0%,
    100% {
      box-shadow: 0 0 0 0 transparent;
    }
    50% {
      box-shadow: 0 0 7px 0 var(--yellow);
    }
  }
  .attn-nm {
    flex-shrink: 0;
    max-width: 42%;
    overflow: hidden;
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .attn-ws {
    flex-shrink: 0;
    color: var(--muted);
    font-size: 0.68rem;
  }
  .attn-lbl {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    color: var(--muted);
    text-align: right;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .attn-time {
    flex-shrink: 0;
    min-width: 30px;
    text-align: right;
    color: var(--text-2);
    font-variant-numeric: tabular-nums;
  }

  .entry {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 8px 10px 4px;
    text-align: left;
    color: var(--text);
    background: none;
    border: none;
    border-left: 3px solid transparent;
    cursor: pointer;
  }
  .entry:hover {
    background: var(--surface-2);
  }
  .entry.active {
    border-left-color: var(--accent);
  }
  .name {
    font-size: 0.85rem;
    font-weight: 700;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .rename {
    width: 100%;
    font-size: 0.8rem;
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--accent);
    border-radius: 4px;
    padding: 1px 4px;
  }
  .panes {
    padding-bottom: 4px;
  }
  .pane-entry {
    display: flex;
    flex-direction: column;
    gap: 1px;
    width: 100%;
    padding: 4px 10px 4px 22px;
    text-align: left;
    color: var(--text-2);
    background: none;
    border: none;
    border-left: 3px solid transparent;
    cursor: pointer;
  }
  .pane-entry:hover {
    background: var(--surface-2);
  }
  .pane-entry.active {
    background: var(--surface-3);
    border-left-color: var(--accent);
  }
  .pane-name {
    font-size: 0.8rem;
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .badge {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--info);
    flex-shrink: 0;
  }
  .status {
    font-size: 0.65rem;
    padding: 0 6px;
    border-radius: 8px;
    flex-shrink: 0;
    font-weight: 600;
  }
  .status.processing {
    color: var(--red);
    background: color-mix(in srgb, var(--red) 15%, transparent);
  }
  .status.processed {
    color: var(--green);
    background: color-mix(in srgb, var(--green) 15%, transparent);
  }
  .status.idle {
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 15%, transparent);
  }
  .status.waiting {
    color: var(--yellow);
    background: color-mix(in srgb, var(--yellow) 15%, transparent);
  }
  .pane-detail {
    display: flex;
    gap: 6px;
    font-size: 0.7rem;
    color: var(--muted);
    overflow: hidden;
    white-space: nowrap;
  }
  .branch {
    color: var(--green);
    flex-shrink: 0;
  }
  .cwd {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ports {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin-left: auto;
  }
  .port {
    font-size: 0.7rem;
    padding: 0 5px;
    color: var(--info);
    background: color-mix(in srgb, var(--info) 12%, transparent);
    border: 1px solid color-mix(in srgb, var(--info) 35%, transparent);
    border-radius: 8px;
  }
  .port:hover {
    background: color-mix(in srgb, var(--info) 30%, transparent);
  }
  .add {
    margin: 8px;
    padding: 7px;
    font-size: 0.8rem;
    color: var(--text);
    background: var(--surface-3);
    border: 1px dashed var(--border-2);
    border-radius: 6px;
    cursor: pointer;
  }
  .add:hover {
    background: var(--surface-4);
  }
  .theme-control {
    position: relative;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 10px 2px;
    font-size: 0.75rem;
    color: var(--muted);
    border-top: 1px solid var(--border);
  }
  .theme-btn {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 3px 6px;
    font-size: 0.75rem;
    color: var(--text);
    background: var(--surface-3);
    border: 1px solid var(--border-2);
    border-radius: 4px;
    cursor: pointer;
  }
  .theme-btn:hover {
    background: var(--border-2);
  }
  .theme-name {
    flex: 1;
    min-width: 0;
    text-align: left;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .caret {
    color: var(--muted);
  }
  .swatch {
    width: 12px;
    height: 12px;
    border-radius: 3px;
    border: 1px solid var(--border-2);
    flex-shrink: 0;
  }
  .theme-menu {
    position: absolute;
    left: 10px;
    right: 10px;
    bottom: calc(100% + 2px);
    z-index: 1000;
    display: flex;
    flex-direction: column;
    padding: 0.25rem;
    background: var(--surface-2);
    border: 1px solid var(--border-2);
    border-radius: 6px;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.5);
  }
  .theme-menu button {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0.35rem 0.5rem;
    text-align: left;
    font-size: 0.8rem;
    color: var(--text);
    background: none;
    border: none;
    border-radius: 4px;
    cursor: pointer;
  }
  .theme-menu button:hover {
    background: var(--border-2);
  }
  .theme-menu button.selected {
    color: var(--accent);
    font-weight: 700;
  }
  .font-control {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px 10px;
    font-size: 0.75rem;
    color: var(--muted);
  }
  .font-label {
    margin-right: auto;
  }
  .font-size {
    min-width: 34px;
    text-align: center;
    color: var(--text-2);
  }
  .font-control button {
    width: 22px;
    height: 20px;
    color: var(--text);
    background: var(--surface-3);
    border: 1px solid var(--border-2);
    border-radius: 4px;
    cursor: pointer;
    line-height: 1;
  }
  .font-control button:hover {
    background: var(--border-2);
  }
  .ctx-menu {
    position: fixed;
    z-index: 1000;
    display: flex;
    flex-direction: column;
    min-width: 10rem;
    padding: 0.25rem;
    background: var(--surface-2);
    border: 1px solid var(--border-2);
    border-radius: 6px;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.5);
  }
  .ctx-menu button {
    padding: 0.4rem 0.75rem;
    text-align: left;
    color: var(--text);
    background: none;
    border: none;
    border-radius: 4px;
    cursor: pointer;
    font-size: 0.85rem;
  }
  .ctx-menu button:hover {
    background: var(--border-2);
  }
</style>
