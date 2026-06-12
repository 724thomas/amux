<script lang="ts">
  // Vertical list: workspaces with their terminals nested beneath
  // (워크스페이스 1 → 터미널 1, 터미널 2 ...). Mouse-first: click to switch,
  // drag to reorder workspaces, right-click to rename/close, port chips
  // open the browser, + creates a workspace.
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
  import { app, paneInfo } from "./state.svelte";

  const snapshot = $derived(app.snapshot);

  type MenuTarget =
    | { kind: "workspace"; id: WorkspaceId; name: string }
    | { kind: "pane"; id: PaneId; name: string };

  let menu = $state<{ x: number; y: number; target: MenuTarget } | null>(null);
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

<svelte:window onclick={() => (menu = null)} />

<nav class="sidebar">
  <ul class="workspaces">
    {#each snapshot?.workspaces ?? [] as ws, index (ws.id)}
      {@const ports = wsPorts(ws)}
      {@const isActiveWs = ws.id === snapshot?.active_workspace}
      <li>
        <button
          class="entry"
          class:active={isActiveWs}
          draggable={renaming?.id !== ws.id}
          onclick={() => void focusWorkspace(ws.id)}
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
                onclick={() => void focusPane(paneId)}
                oncontextmenu={(e) =>
                  openMenu(e, { kind: "pane", id: paneId, name: pane?.name ?? "" })}
              >
                {#if renaming?.kind === "pane" && renaming.id === paneId}
                  {@render renameInput()}
                {:else}
                  <span class="pane-name">{pane?.name ?? "터미널"}</span>
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
    width: 230px;
    flex-shrink: 0;
    background: #1a1b26;
    border-right: 1px solid #2a2e42;
    overflow-y: auto;
  }
  ul {
    list-style: none;
  }
  .workspaces {
    flex: 1;
  }
  .entry {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 8px 10px 4px;
    text-align: left;
    color: #c0caf5;
    background: none;
    border: none;
    border-left: 3px solid transparent;
    cursor: pointer;
  }
  .entry:hover {
    background: #1f2335;
  }
  .entry.active {
    border-left-color: #7aa2f7;
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
    color: #c0caf5;
    background: #16161e;
    border: 1px solid #7aa2f7;
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
    color: #a9b1d6;
    background: none;
    border: none;
    border-left: 3px solid transparent;
    cursor: pointer;
  }
  .pane-entry:hover {
    background: #1f2335;
  }
  .pane-entry.active {
    background: #24283b;
    border-left-color: #7aa2f7;
  }
  .pane-name {
    font-size: 0.8rem;
  }
  .pane-detail {
    display: flex;
    gap: 6px;
    font-size: 0.7rem;
    color: #565f89;
    overflow: hidden;
    white-space: nowrap;
  }
  .branch {
    color: #9ece6a;
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
    color: #7dcfff;
    background: rgba(125, 207, 255, 0.12);
    border: 1px solid rgba(125, 207, 255, 0.35);
    border-radius: 8px;
  }
  .port:hover {
    background: rgba(125, 207, 255, 0.3);
  }
  .add {
    margin: 8px;
    padding: 7px;
    font-size: 0.8rem;
    color: #c0caf5;
    background: #24283b;
    border: 1px dashed #3b4261;
    border-radius: 6px;
    cursor: pointer;
  }
  .add:hover {
    background: #2f334d;
  }
  .ctx-menu {
    position: fixed;
    z-index: 1000;
    display: flex;
    flex-direction: column;
    min-width: 10rem;
    padding: 0.25rem;
    background: #1f2335;
    border: 1px solid #3b4261;
    border-radius: 6px;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.5);
  }
  .ctx-menu button {
    padding: 0.4rem 0.75rem;
    text-align: left;
    color: #c0caf5;
    background: none;
    border: none;
    border-radius: 4px;
    cursor: pointer;
    font-size: 0.85rem;
  }
  .ctx-menu button:hover {
    background: #3b4261;
  }
</style>
