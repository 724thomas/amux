<script lang="ts">
  // Vertical list: workspaces with their terminals nested beneath
  // (워크스페이스 1 → 터미널 1, 터미널 2 ...). Mouse-first: click to switch,
  // drag to reorder workspaces, right-click to rename/close, port chips
  // open the browser, + creates a workspace.
  import { openUrl } from "@tauri-apps/plugin-opener";
  import {
    clearNotificationHistory,
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
  import { app, focusTerm, paneInfo } from "./state.svelte";
  import { adjustFontSize, setNotifHeight, setTheme, settings } from "./settings.svelte";
  import { THEMES, themeById } from "./themes";

  const snapshot = $derived(app.snapshot);

  // Animated "processing" indicator: processing. → .. → ... → . (cycles).
  let dots = $state(1);
  $effect(() => {
    const t = setInterval(() => (dots = (dots % 3) + 1), 450);
    return () => clearInterval(t);
  });

  // Drag-resize the notification panel; growing it eats into the workspace list.
  let notifDrag = $state<{ y: number; h: number } | null>(null);

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

  function fmtTime(atMs: number): string {
    const d = new Date(atMs);
    const hh = String(d.getHours()).padStart(2, "0");
    const mm = String(d.getMinutes()).padStart(2, "0");
    return `${hh}:${mm}`;
  }

  function notifIcon(kind: string): string {
    switch (kind) {
      case "attention":
        return "●";
      case "done":
        return "✓";
      case "bell":
        return "♪";
      default:
        return "·";
    }
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

  <!-- 알림 패널 높이 조절 핸들 (위로 드래그하면 패널이 커지고 워크스페이스 영역이 줄어듦) -->
  <div
    class="notif-resizer"
    class:dragging={notifDrag !== null}
    role="separator"
    aria-orientation="horizontal"
    onpointerdown={(e) => {
      notifDrag = { y: e.clientY, h: settings.notifHeight };
      (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    }}
    onpointermove={(e) => {
      if (notifDrag) setNotifHeight(notifDrag.h + (notifDrag.y - e.clientY));
    }}
    onpointerup={() => (notifDrag = null)}
    onpointercancel={() => (notifDrag = null)}
  ></div>

  <!-- 하단: 알림 히스토리 -->
  <div class="notif-panel" style="height: {settings.notifHeight}px">
    <div class="notif-head">
      <span>알림</span>
      {#if (snapshot?.notifications.length ?? 0) > 0}
        <button class="notif-clear" onclick={() => void clearNotificationHistory()}>지우기</button>
      {/if}
    </div>
    <ul class="notif-list">
      {#each snapshot?.notifications ?? [] as n (n.at_ms + n.pane)}
        <li>
          <button class="notif-entry" onclick={() => void focusPane(n.pane).catch(() => {})}>
            <span class="notif-line">
              <span class="notif-icon {n.kind}">{notifIcon(n.kind)}</span>
              <span class="notif-pane">{n.pane_name}</span>
              <span class="notif-time">{fmtTime(n.at_ms)}</span>
            </span>
            <span class="notif-msg">{n.body ?? n.title ?? ""}</span>
          </button>
        </li>
      {:else}
        <li class="notif-empty">알림 없음</li>
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
  .notif-resizer {
    flex: 0 0 5px;
    cursor: row-resize;
    background: var(--border);
    touch-action: none;
  }
  .notif-resizer:hover,
  .notif-resizer.dragging {
    background: var(--accent);
  }
  .notif-panel {
    display: flex;
    flex-direction: column;
    flex-shrink: 0;
    min-height: 0;
  }
  .notif-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 6px 10px 4px;
    font-size: 0.75rem;
    font-weight: 700;
    color: var(--muted);
  }
  .notif-clear {
    font-size: 0.7rem;
    color: var(--muted);
    background: none;
    border: none;
    cursor: pointer;
  }
  .notif-clear:hover {
    color: var(--text);
  }
  .notif-list {
    overflow-y: auto;
  }
  .notif-empty {
    padding: 4px 10px 8px;
    font-size: 0.72rem;
    color: var(--border-2);
  }
  .notif-entry {
    display: flex;
    flex-direction: column;
    gap: 1px;
    width: 100%;
    padding: 4px 10px;
    text-align: left;
    background: none;
    border: none;
    cursor: pointer;
  }
  .notif-entry:hover {
    background: var(--surface-2);
  }
  .notif-line {
    display: flex;
    gap: 6px;
    align-items: baseline;
    font-size: 0.72rem;
  }
  .notif-icon.attention {
    color: var(--red);
  }
  .notif-icon.done {
    color: var(--green);
  }
  .notif-icon.bell {
    color: var(--yellow);
  }
  .notif-pane {
    color: var(--text-2);
    font-weight: 600;
  }
  .notif-time {
    margin-left: auto;
    color: var(--border-2);
  }
  .notif-msg {
    font-size: 0.72rem;
    color: var(--info);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
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
