<script lang="ts">
  // Vertical list: workspaces with their tabs nested beneath
  // (워크스페이스 1 → 탭 1, 탭 2 ...). A tab is the named unit — the panes
  // split inside it are shown in the tab bar above the terminals, not here.
  // Mouse-first: click to switch, drag to reorder workspaces, right-click to
  // rename/close, port chips open the browser, + creates a workspace. The
  // bottom panel is the "지금 봐야 할 에이전트" triage list.
  import { openUrl } from "@tauri-apps/plugin-opener";
  import {
    closeTab,
    closeWorkspace,
    createWorkspace,
    focusPane,
    focusTab,
    focusWorkspace,
    moveWorkspace,
    renameTab,
    renameWorkspace,
    type TabId,
    type TabInfo,
    type WorkspaceId,
    type WorkspaceInfo,
  } from "./ipc";
  import {
    app,
    attentionItems,
    clock,
    focusTerm,
    paneInfo,
    tabHasBadge,
    tabPanes,
    tabStatus,
    wsCreate,
  } from "./state.svelte";
  import {
    adjustFontSize,
    setNotifHeight,
    setTheme,
    settings,
    toggleShowLastInput,
    toggleShowComposer,
  } from "./settings.svelte";
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
    | { kind: "tab"; id: TabId; name: string };

  let menu = $state<{ x: number; y: number; target: MenuTarget } | null>(null);
  let themeMenuOpen = $state(false);
  let renaming = $state<{ kind: "workspace" | "tab"; id: string } | null>(null);
  let renameValue = $state("");
  let draggedId = $state<WorkspaceId | null>(null);

  // New-workspace prompt (opened by the "+" button or Ctrl+Shift+N via the
  // shared wsCreate flag). Two steps, because a workspace is born with its
  // first tab already in it: ask the workspace title, then that tab's title —
  // the same "name it before it exists" beat as Ctrl+T. Nothing is created
  // until the second Enter; a blank name at either step falls back to the
  // engine's auto-name (`워크스페이스 N` / `탭 1`).
  let wsStep = $state<"workspace" | "tab">("workspace");
  // One input element serves both steps — only its placeholder and the value it
  // holds change. Swapping in a *second* element would unmount the focused one,
  // and its `onblur` (which cancels) could fire on the way out and kill the
  // prompt mid-flow.
  let wsDraft = $state("");
  let newWsName = $state("");
  let newWsInput = $state<HTMLInputElement | null>(null);
  // A plain `autofocus` attribute doesn't fire on a dynamically-mounted input in
  // this webview (Palette hits the same issue), so focus it explicitly once the
  // prompt opens — works whether opened by mouse ("+") or keyboard (Ctrl+Shift+N).
  $effect(() => {
    if (wsCreate.open) newWsInput?.focus();
  });
  function startCreateWorkspace() {
    wsDraft = "";
    newWsName = "";
    wsStep = "workspace";
    wsCreate.open = true;
  }
  /** Enter on step 1 banks the workspace title and moves to the tab title;
   *  Enter on step 2 is what actually creates both. */
  function advanceCreateWorkspace() {
    if (wsStep === "workspace") {
      newWsName = wsDraft.trim();
      wsDraft = "";
      wsStep = "tab";
      return;
    }
    const name = newWsName;
    const tabName = wsDraft.trim();
    cancelCreateWorkspace();
    void createWorkspace(name || undefined, tabName || undefined);
  }
  function cancelCreateWorkspace() {
    wsCreate.open = false;
    wsDraft = "";
    newWsName = "";
    wsStep = "workspace";
  }

  /// Branch / cwd shown on a tab row come from the pane the tab is focused on
  /// — with one pane that IS the tab, and with a split it is the one the user
  /// last touched, which is the useful one to surface.
  function tabDetail(tab: TabInfo): { branch: string | null; cwd: string | null } {
    const pane = paneInfo(tab.active_pane ?? tabPanes(tab)[0] ?? "");
    return { branch: pane?.meta.git_branch ?? null, cwd: pane?.meta.cwd ?? null };
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
      else void renameTab(renaming.id, name);
    }
    renaming = null;
  }

  function closeTarget(target: MenuTarget) {
    menu = null;
    if (target.kind === "workspace") void closeWorkspace(target.id);
    else void closeTab(target.id);
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
            // hand the keyboard to the visible tab's terminal right now.
            focusTerm(ws.tabs.find((t) => t.id === ws.active_tab)?.active_pane);
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
          {#each ws.tabs as tab (tab.id)}
            {@const status = tabStatus(tab)}
            {@const detail = tabDetail(tab)}
            {@const paneCount = tabPanes(tab).length}
            <li>
              <button
                class="pane-entry"
                class:active={isActiveWs && tab.id === ws.active_tab}
                onclick={() => {
                  void focusTab(tab.id);
                  focusTerm(tab.active_pane);
                }}
                oncontextmenu={(e) => openMenu(e, { kind: "tab", id: tab.id, name: tab.name })}
              >
                {#if renaming?.kind === "tab" && renaming.id === tab.id}
                  {@render renameInput()}
                {:else}
                  <span class="pane-name">
                    {tab.name}
                    <!-- A split tab holds more than one terminal; say how many
                         so the single status chip isn't read as the whole story. -->
                    {#if paneCount > 1}<span class="split-count">◫{paneCount}</span>{/if}
                    {#if status}
                      <span class="status {status}">
                        {#if status === "processing"}
                          {"processing" + ".".repeat(dots)}
                        {:else if status === "done"}
                          DONE
                        {:else}
                          {status}
                        {/if}
                      </span>
                    {/if}
                    {#if tabHasBadge(tab)}<span class="badge"></span>{/if}
                  </span>
                  <span class="pane-detail">
                    {#if detail.branch}<span class="branch">⎇ {detail.branch}</span>{/if}
                    <span class="cwd">{shortCwd(detail.cwd)}</span>
                  </span>
                {/if}
              </button>
            </li>
          {/each}
        </ul>
      </li>
    {/each}
  </ul>
  {#if wsCreate.open}
    <!-- Step 1 asks for the workspace title, step 2 for its first tab's.
         One input, swapped in place, so the sidebar doesn't jump. -->
    <div class="add-steps">
      <span class="add-step">{wsStep === "workspace" ? "1/2 워크스페이스" : "2/2 첫 탭"}</span>
      {#if wsStep === "tab" && newWsName}
        <span class="add-done">{newWsName}</span>
      {/if}
    </div>
    <input
      class="add-input"
      bind:this={newWsInput}
      placeholder={wsStep === "workspace"
        ? "워크스페이스 이름 (Enter 다음 · Esc 취소)"
        : "첫 탭 이름 (Enter 생성 · Esc 취소)"}
      bind:value={wsDraft}
      onblur={cancelCreateWorkspace}
      onkeydown={(e) => {
        if (e.key === "Enter") advanceCreateWorkspace();
        else if (e.key === "Escape") cancelCreateWorkspace();
        e.stopPropagation();
      }}
      onclick={(e) => e.stopPropagation()}
    />
  {:else}
    <button class="add" onclick={startCreateWorkspace}>+ 새 워크스페이스</button>
  {/if}

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

  <!-- 직전 명령 표시 토글 (폰트 바로 아래) — 각 pane 맨 위에 마지막으로 보낸
       명령을 가로 전체 한 줄 띠로 표시. -->
  <div class="toggle-control" title="각 pane 맨 위에 직전에 보낸 명령을 가로 전체 한 줄로 표시 (길면 끝을 … 로 자르고, 올려두면 전체가 보입니다)">
    <span class="font-label">직전 명령 표시</span>
    <button
      class="switch"
      class:on={settings.showLastInput}
      role="switch"
      aria-checked={settings.showLastInput}
      aria-label="직전 명령 표시 토글"
      onclick={toggleShowLastInput}
    >
      <span class="knob"></span>
    </button>
  </div>

  <!-- 하단 고정 입력창 토글 — pane 아래에 프롬프트 작성칸을 붙여, 출력을
       스크롤해 읽는 중에도 화면이 맨 아래로 튀지 않게 한다. -->
  <div
    class="toggle-control"
    title="pane 아래에 프롬프트 입력칸 고정 (Ctrl+Shift+E 로 입력칸↔터미널 이동) — 여기 타이핑하면 터미널 스크롤이 움직이지 않습니다"
  >
    <span class="font-label">하단 입력창</span>
    <button
      class="switch"
      class:on={settings.showComposer ?? true}
      role="switch"
      aria-checked={settings.showComposer ?? true}
      aria-label="하단 입력창 토글"
      onclick={toggleShowComposer}
    >
      <span class="knob"></span>
    </button>
  </div>
</nav>

{#if menu}
  {@const target = menu.target}
  <div class="ctx-menu" style="left: {menu.x}px; top: {menu.y}px">
    <button onclick={() => startRename(target)}>이름 변경</button>
    <button onclick={() => closeTarget(target)}>
      {target.kind === "workspace" ? "워크스페이스 닫기" : "탭 닫기"}
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
  /* "이 탭 안에 터미널이 N개" — split tabs only. */
  .split-count {
    flex-shrink: 0;
    padding: 0 4px;
    font-size: 0.65rem;
    color: var(--muted);
    background: color-mix(in srgb, var(--text) 8%, transparent);
    border-radius: 6px;
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
  /* Pinned by the user, not derived like the other four — the outline says
     "this one is held here on purpose". */
  .status.done {
    color: var(--done);
    background: color-mix(in srgb, var(--done) 15%, transparent);
    border: 1px solid color-mix(in srgb, var(--done) 45%, transparent);
    padding: 0 5px;
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
  /* "1/2 워크스페이스" → "2/2 첫 탭" progress line above the input, with the
     name already entered echoed back so step 2 isn't context-free. */
  .add-steps {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 8px 8px 0;
    font-size: 0.68rem;
    color: var(--muted);
  }
  .add-done {
    padding: 1px 6px;
    color: var(--text);
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    border-radius: 6px;
  }
  .add-input {
    margin: 6px 8px 8px;
    padding: 7px;
    font-size: 0.8rem;
    color: var(--text);
    background: var(--surface-3);
    border: 1px solid var(--accent);
    border-radius: 6px;
    outline: none;
  }
  .add-input::placeholder {
    color: var(--muted);
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
    padding: 4px 10px 6px;
    font-size: 0.75rem;
    color: var(--muted);
  }
  .toggle-control {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 10px 10px;
    font-size: 0.75rem;
    color: var(--muted);
  }
  .switch {
    position: relative;
    flex-shrink: 0;
    width: 34px;
    height: 18px;
    padding: 0;
    background: var(--surface-3);
    border: 1px solid var(--border-2);
    border-radius: 999px;
    cursor: pointer;
    transition: background 0.15s;
  }
  .switch.on {
    background: var(--accent);
    border-color: var(--accent);
  }
  .switch .knob {
    position: absolute;
    top: 1px;
    left: 1px;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--text-2);
    transition: transform 0.15s;
  }
  .switch.on .knob {
    transform: translateX(16px);
    background: var(--bg);
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
