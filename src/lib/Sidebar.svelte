<script lang="ts">
  // Vertical workspace list: name, git branch, cwd, listening ports.
  // Mouse-first: click to switch, drag to reorder, right-click to
  // rename/close, port chips open the browser, + creates a workspace.
  import { openUrl } from "@tauri-apps/plugin-opener";
  import {
    closeWorkspace,
    createWorkspace,
    focusWorkspace,
    moveWorkspace,
    renameWorkspace,
    type WorkspaceId,
    type WorkspaceInfo,
  } from "./ipc";
  import { app } from "./state.svelte";

  const snapshot = $derived(app.snapshot);

  let menu = $state<{ x: number; y: number; workspace: WorkspaceId } | null>(null);
  let renaming = $state<WorkspaceId | null>(null);
  let renameValue = $state("");
  let draggedId = $state<WorkspaceId | null>(null);

  function wsPorts(ws: WorkspaceInfo): number[] {
    const ports = new Set<number>();
    for (const pane of snapshot?.panes ?? []) {
      if (pane.workspace !== ws.id) continue;
      for (const port of pane.meta.listening_ports) ports.add(port);
    }
    return [...ports].sort((a, b) => a - b);
  }

  function wsMeta(ws: WorkspaceInfo) {
    return snapshot?.panes.find((p) => p.id === ws.active_pane)?.meta ?? null;
  }

  function shortCwd(cwd: string | null): string {
    if (!cwd) return "";
    const parts = cwd.split("/").filter(Boolean);
    return parts.length > 2 ? "…/" + parts.slice(-2).join("/") : cwd;
  }

  function startRename(id: WorkspaceId, current: string) {
    menu = null;
    renaming = id;
    renameValue = current;
  }

  function commitRename() {
    if (renaming && renameValue.trim()) {
      void renameWorkspace(renaming, renameValue.trim());
    }
    renaming = null;
  }

  function onDrop(targetIndex: number) {
    if (draggedId) void moveWorkspace(draggedId, targetIndex);
    draggedId = null;
  }
</script>

<svelte:window onclick={() => (menu = null)} />

<nav class="sidebar">
  <ul>
    {#each snapshot?.workspaces ?? [] as ws, index (ws.id)}
      {@const meta = wsMeta(ws)}
      {@const ports = wsPorts(ws)}
      <li>
        <button
          class="entry"
          class:active={ws.id === snapshot?.active_workspace}
          draggable={renaming !== ws.id}
          onclick={() => void focusWorkspace(ws.id)}
          oncontextmenu={(e) => {
            e.preventDefault();
            menu = { x: e.clientX, y: e.clientY, workspace: ws.id };
          }}
          ondragstart={() => (draggedId = ws.id)}
          ondragover={(e) => e.preventDefault()}
          ondrop={(e) => {
            e.preventDefault();
            onDrop(index);
          }}
        >
          {#if renaming === ws.id}
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
          {:else}
            <span class="name">{ws.name}</span>
          {/if}
          <span class="detail">
            {#if meta?.git_branch}<span class="branch">⎇ {meta.git_branch}</span>{/if}
            <span class="cwd">{shortCwd(meta?.cwd ?? null)}</span>
          </span>
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
      </li>
    {/each}
  </ul>
  <button class="add" onclick={() => void createWorkspace()}>+ 새 워크스페이스</button>
</nav>

{#if menu}
  {@const target = menu.workspace}
  {@const name = snapshot?.workspaces.find((w) => w.id === target)?.name ?? ""}
  <div class="ctx-menu" style="left: {menu.x}px; top: {menu.y}px">
    <button onclick={() => startRename(target, name)}>이름 변경</button>
    <button
      onclick={() => {
        menu = null;
        void closeWorkspace(target);
      }}>워크스페이스 닫기</button
    >
  </div>
{/if}

<style>
  .sidebar {
    display: flex;
    flex-direction: column;
    width: 220px;
    flex-shrink: 0;
    background: #1a1b26;
    border-right: 1px solid #2a2e42;
    overflow-y: auto;
  }
  ul {
    flex: 1;
    list-style: none;
  }
  .entry {
    display: flex;
    flex-direction: column;
    gap: 2px;
    width: 100%;
    padding: 8px 10px;
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
    background: #24283b;
    border-left-color: #7aa2f7;
  }
  .name {
    font-size: 0.85rem;
    font-weight: 600;
  }
  .rename {
    font-size: 0.85rem;
    color: #c0caf5;
    background: #16161e;
    border: 1px solid #7aa2f7;
    border-radius: 4px;
    padding: 1px 4px;
  }
  .detail {
    display: flex;
    gap: 6px;
    font-size: 0.72rem;
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
