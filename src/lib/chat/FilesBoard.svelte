<script lang="ts">
  // The files pop-out window (window label "files", see src-tauri/src/popout.rs):
  // the files panels that ⧉ moved out of the main window, side by side in one
  // window. This page is a separate copy of the app's front end, so it builds
  // what each panel needs itself:
  //   - which panes are out, and the main window's theme: `files:state`
  //   - names, folders and statuses of the panes: `state:snapshot`
  //   - a chat's tool calls, typed while Claude writes them: `chat:event`, read
  //     into a chat model of its own, for every chat (out here or not) while
  //     this window is open: a chat whose panel comes out in the middle of a
  //     change (the main window sends it here when one starts) has the call
  //     from its start. A call already under way when this window opened is
  //     played back from disk instead.
  //   - a terminal's changes: played back from disk when its status says a
  //     tool call finished (statusFeed.ts), as beside the terminal.
  import { onDestroy, onMount, untrack } from "svelte";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import FilesPanel from "./FilesPanel.svelte";
  import {
    applyExit,
    applyLine,
    applyStderr,
    createModel,
    settleWatched,
    type ChatModel,
    type FileFeed,
  } from "./reducer";
  import { emptyFeed, statusFeed } from "./statusFeed";
  import {
    filesPop,
    filesState,
    getSnapshot,
    type ChatEventPayload,
    type Look,
    type PaneId,
    type PopoutState,
    type Snapshot,
  } from "../ipc";
  import { layoutPanes } from "../state.svelte";
  import { settings } from "../settings.svelte";
  import { themeById } from "../themes";

  /** Narrower cells than this stack in more rows (and the window scrolls). */
  const MIN_CELL_W = 360;
  /** The cell shape the column count aims for (code reads better wide). */
  const CELL_RATIO = 1.3;

  let panes = $state<PaneId[]>([]);
  let look = $state<Look | null>(null);
  let snap = $state<Snapshot | null>(null);
  let zoomed = $state<PaneId | null>(null);
  /** The first answers (the list, the panes) are in. */
  let loaded = $state(false);

  // A chat pane's model, rebuilt here from Claude's output. `gen` is the spawn
  // it follows (a new conversation is a new spawn); -1 = none seen yet.
  const chats = $state<Record<PaneId, { gen: number; model: ChatModel }>>({});
  // A terminal pane's feed, driven by its status.
  const terms = $state<Record<PaneId, FileFeed>>({});
  const drivers = new Map<PaneId, ReturnType<typeof statusFeed>>();

  const isChat = (pane: PaneId) => snap?.panes.find((p) => p.id === pane)?.chat === true;

  /** Make or drop feeds to match the panes that are out; runs after every change. */
  const docking = new Set<PaneId>();
  function sync() {
    for (const pane of panes) {
      const info = snap?.panes.find((p) => p.id === pane);
      if (snap && !info) {
        // Its pane was closed: nothing to put back, just leave the list.
        if (!docking.has(pane)) {
          docking.add(pane);
          void filesPop(pane, false, null).catch(() => docking.delete(pane));
        }
        continue;
      }
      if (info?.chat) {
        chats[pane] ??= { gen: -1, model: createModel() };
      } else if (info) {
        if (!terms[pane]) {
          terms[pane] = emptyFeed();
          drivers.set(pane, statusFeed(terms[pane]));
        }
        drivers.get(pane)?.update(info.status);
      }
    }
    // Every chat keeps its model (a panel may come out at any moment); a
    // pane that closed or went back to its terminal drops it.
    for (const pane of Object.keys(chats)) {
      if (snap && !snap.panes.some((p) => p.id === pane && p.chat)) delete chats[pane];
    }
    for (const pane of Object.keys(terms)) {
      if (panes.includes(pane) && !isChat(pane)) continue;
      drivers.get(pane)?.dispose();
      drivers.delete(pane);
      delete terms[pane];
    }
    if (zoomed && !panes.includes(zoomed)) zoomed = null;
  }

  function takeState(s: PopoutState) {
    panes = s.panes;
    look = s.look;
    for (const pane of [...docking]) if (!panes.includes(pane)) docking.delete(pane);
    sync();
  }

  function onChat(ev: ChatEventPayload) {
    // A pane known to be gone or not a chat (its last lines, after it closed).
    if (snap && !snap.panes.some((p) => p.id === ev.pane && p.chat)) return;
    let s = chats[ev.pane];
    if (!s) {
      chats[ev.pane] = { gen: ev.gen, model: createModel() };
      s = chats[ev.pane];
    } else if (ev.gen > s.gen) {
      // First line seen, or a new spawn (a new conversation): a fresh model.
      if (s.gen !== -1) s.model = createModel();
      s.gen = ev.gen;
    } else if (ev.gen < s.gen) {
      return; // the end of a spawn that has already been replaced
    }
    const m = s.model;
    const was = m.status;
    try {
      if (ev.kind === "line") {
        const obj = JSON.parse(ev.data);
        applyLine(m, obj);
        settleWatched(m, obj?.type === "result");
      } else if (ev.kind === "stderr") applyStderr(m, ev.data);
      else if (ev.kind === "exit") applyExit(m, ev.data);
    } catch (e) {
      // The rest of a message that began before this window was listening.
      console.debug("[amux files] skipped a chat line", e);
    }
    // A turn started: following comes back on (the main window counts the
    // user's messages instead, which this window does not see).
    if (m.status === "working" && was !== "working" && was !== "waiting") m.userSeq++;
  }

  const unlisten: UnlistenFn[] = [];
  let dead = false;
  onMount(() => {
    void (async () => {
      // Listen first, then ask: a change landing in between is not lost.
      const offs = await Promise.all([
        listen<PopoutState>("files:state", (e) => takeState(e.payload)),
        listen<Snapshot>("state:snapshot", (e) => {
          snap = e.payload;
          sync();
        }),
        listen<ChatEventPayload>("chat:event", (e) => onChat(e.payload)),
      ]);
      if (dead) return offs.forEach((off) => off());
      unlisten.push(...offs);
      const [s, state] = await Promise.all([getSnapshot(), filesState()]);
      snap ??= s;
      takeState(state);
      loaded = true;
    })().catch((e) => console.error("[amux files] start", e));
  });
  onDestroy(() => {
    dead = true;
    unlisten.forEach((off) => off());
    for (const d of drivers.values()) d.dispose();
  });

  // The main window's theme (sent with each change), else this page's own copy.
  $effect(() => {
    const { chrome } = themeById(look?.theme ?? settings.theme);
    for (const [key, value] of Object.entries(chrome)) {
      document.documentElement.style.setProperty(key, value);
    }
  });
  const fontSize = $derived(look?.fontSize ?? settings.fontSize);

  // The session the main window is on: its focused pane (the active pane of
  // the active tab of the active workspace, as activePane() in state.svelte).
  // The engine records every switch there (workspace, tab, a click in a pane)
  // and sends a new snapshot, so this follows each one.
  const current = $derived.by(() => {
    const ws = snap?.workspaces.find((w) => w.id === snap?.active_workspace);
    const tab = ws?.tabs.find((t) => t.id === ws.active_tab);
    return tab?.active_pane ?? null;
  });
  /** Its cell, if its panel is in this window: drawn with the accent border. */
  const currentCell = $derived(current && panes.includes(current) ? current : null);
  // Each time it changes (a switch in the main window, or the current session's
  // panel arriving), its border flashes once; `seq` alternates between two
  // identical animations so a repeat restarts it.
  let flash = $state<{ pane: PaneId; seq: number } | null>(null);
  $effect(() => {
    const pane = currentCell;
    if (!pane) return;
    untrack(() => {
      flash = { pane, seq: (flash?.seq ?? 0) + 1 };
      // Out of sight in a scrolled (narrow) window: bring it in.
      document
        .querySelector(`.board .cell[data-pane="${pane}"]`)
        ?.scrollIntoView({ block: "nearest", inline: "nearest" });
    });
  });

  function labelOf(pane: PaneId): string {
    const info = snap?.panes.find((p) => p.id === pane);
    const ws = info && snap?.workspaces.find((w) => w.id === info.workspace);
    const tab = info && ws?.tabs.find((t) => t.id === info.tab);
    if (!info || !ws || !tab) return "세션";
    const inTab = layoutPanes(tab.layout);
    const ord = inTab.length > 1 ? ` #${inTab.indexOf(pane) + 1}` : "";
    const mode = info.chat ? "채팅" : "터미널";
    const what =
      info.status === "processing" ? ", 작업 중" : info.status === "waiting" ? ", 응답 기다림" : "";
    return `${ws.name} / ${tab.name}${ord} (${mode}${what})`;
  }

  interface Cell {
    pane: PaneId;
    feed: FileFeed;
    cwd: string | null;
    label: string;
  }
  const cells = $derived(
    panes.flatMap((pane): Cell[] => {
      const info = snap?.panes.find((p) => p.id === pane);
      if (!info) return [];
      const chat = info.chat ? chats[pane] : undefined;
      const feed = info.chat ? chat?.model : terms[pane];
      if (!feed) return [];
      return [{ pane, feed, cwd: chat?.model.cwd ?? info.meta.cwd ?? null, label: labelOf(pane) }];
    }),
  );

  // Columns: the count whose cells come closest to CELL_RATIO, never narrower
  // than MIN_CELL_W (more rows instead, and the window scrolls).
  let boardW = $state(0);
  let boardH = $state(0);
  function columnsFor(n: number, w: number, h: number): number {
    if (n <= 1 || w <= 0 || h <= 0) return 1;
    let best = 1;
    let bestScore = Infinity;
    for (let c = 1; c <= n; c++) {
      const rows = Math.ceil(n / c);
      const cw = w / c;
      if (c > 1 && cw < MIN_CELL_W) break;
      // Empty slots (the last row's cell stretches over them) cost a little.
      const score = Math.abs(Math.log(cw / (h / rows) / CELL_RATIO)) + (rows * c - n) * 0.15;
      if (score < bestScore) {
        best = c;
        bestScore = score;
      }
    }
    return best;
  }
  const shown = $derived(zoomed ? cells.filter((c) => c.pane === zoomed) : cells);
  const cols = $derived(columnsFor(shown.length, boardW, boardH));
  /** The last cell stretches over the empty slots of the last row. */
  function spanOf(i: number): string | undefined {
    const n = shown.length;
    const short = n % cols;
    return short && i === n - 1 ? `span ${cols - short + 1}` : undefined;
  }

  // Double-clicking a cell zooms it, and double-clicking it again splits the
  // window back into cells (what ⤢ and ⤡ do). Not on a control (a button, a
  // tab, the list, the checkbox), where a quick second click means something
  // of its own. Text in the panel cannot be selected (the app turns selection
  // off), so a double-click on it has no other meaning. Listened to on the
  // grid, so one listener serves every cell.
  let gridEl = $state<HTMLElement | null>(null);
  $effect(() => {
    const el = gridEl;
    if (!el) return;
    const onDblClick = (e: MouseEvent) => {
      const target = e.target as HTMLElement | null;
      const cell = target?.closest<HTMLElement>(".cell");
      if (!cell || cells.length < 2) return;
      if (target?.closest("button, input, label, select, textarea, a")) return;
      const pane = cell.dataset.pane as PaneId | undefined;
      if (!pane) return;
      zoomed = zoomed === pane ? null : pane;
    };
    el.addEventListener("dblclick", onDblClick);
    return () => el.removeEventListener("dblclick", onDblClick);
  });

  function dock(pane: PaneId) {
    void filesPop(pane, false, null)
      .then(takeState)
      .catch((e) => console.warn("[amux files] put back", e));
  }

  if (import.meta.env.DEV) {
    (window as unknown as Record<string, unknown>).__amuxBoard = {
      panes: () => [...panes],
      cols: () => cols,
      cells: () =>
        cells.map((c) => ({
          pane: c.pane,
          label: c.label,
          cwd: c.cwd,
          chat: c.pane in chats,
          gen: chats[c.pane]?.gen ?? null,
          status: c.feed.status,
        })),
      zoomed: () => zoomed,
      current: () => currentCell,
    };
  }
</script>

<div class="board" style="--chat-fs: {fontSize}px">
  {#if loaded && cells.length === 0}
    <div class="empty">
      <p>여기로 옮긴 파일 창이 없습니다.</p>
      <p>
        메인 창에서 파일 창 위쪽의 ⧉ 버튼을 누르면 그 세션의 파일 창이 이 창에 한 칸으로 들어옵니다. 여러 세션을
        옮기면 칸이 나뉘어 한눈에 보입니다.
      </p>
    </div>
  {/if}
  <div
    class:gone={cells.length === 0}
    class="grid"
    bind:this={gridEl}
    style:--cols={cols}
    bind:clientWidth={boardW}
    bind:clientHeight={boardH}
  >
    {#each cells as c (c.pane)}
      <!-- `cells` is rebuilt on every snapshot. Through @const (a derived) a
           prop changes only when its value does; passed as c.cwd it would
           change with each new cell object, and the panel would take every
           snapshot for a new folder and start over (no playback). -->
      {@const at = shown.indexOf(c)}
      {@const feed = c.feed}
      {@const cwd = c.cwd}
      {@const label = c.label}
      <div
        class="cell"
        data-pane={c.pane}
        class:gone={at < 0}
        class:current={currentCell === c.pane}
        class:flash-a={flash?.pane === c.pane && flash.seq % 2 === 1}
        class:flash-b={flash?.pane === c.pane && flash.seq % 2 === 0}
        title={currentCell === c.pane ? "메인 창에서 지금 고른 세션" : undefined}
        style:grid-column={at < 0 ? undefined : spanOf(at)}
      >
        <FilesPanel
          model={feed}
          {cwd}
          visible={at >= 0}
          fill
          {label}
          closable={false}
          onclose={() => dock(c.pane)}
          persistKey="popout"
          actions={[
            ...(cells.length > 1
              ? [
                  {
                    icon: zoomed === c.pane ? "⤡" : "⤢",
                    title:
                      zoomed === c.pane
                        ? "다시 칸으로 나눠 보기 (칸을 두 번 눌러도 됩니다)"
                        : "이 칸만 크게 보기 (칸을 두 번 눌러도 됩니다)",
                    on: zoomed === c.pane,
                    run: () => (zoomed = zoomed === c.pane ? null : c.pane),
                  },
                ]
              : []),
            { icon: "↩", title: "이 파일 창을 원래 세션 화면으로 되돌리기", run: () => dock(c.pane) },
          ]}
        />
      </div>
    {/each}
  </div>
</div>

<style>
  .board {
    position: fixed;
    inset: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg);
    color: var(--text);
    font-family: system-ui, sans-serif;
    font-size: var(--chat-fs, 14px);
    line-height: 1.55;
  }
  .grid {
    flex: 1 1 auto;
    min-height: 0;
    display: grid;
    grid-template-columns: repeat(var(--cols, 1), minmax(0, 1fr));
    grid-auto-rows: minmax(260px, 1fr);
    gap: 6px;
    padding: 6px;
    box-sizing: border-box;
    overflow-y: auto;
  }
  .cell {
    position: relative;
    min-width: 0;
    min-height: 0;
    border: 1px solid var(--border);
    border-radius: 6px;
    overflow: hidden;
  }
  /* The cell of the session the main window is on (see `current`): an accent
     border, drawn over its neighbours so the glow is not cut by them. */
  .cell.current {
    z-index: 1;
    border-color: var(--accent);
    box-shadow: 0 0 0 2px var(--accent);
  }
  /* Each switch flashes it once, to catch the eye. */
  .cell.flash-a {
    animation: cell-flash-a 1.2s ease-out;
  }
  .cell.flash-b {
    animation: cell-flash-b 1.2s ease-out;
  }
  @keyframes cell-flash-a {
    0%,
    35% {
      box-shadow:
        0 0 0 2px var(--accent),
        0 0 16px 4px var(--accent);
    }
    100% {
      box-shadow:
        0 0 0 2px var(--accent),
        0 0 0 0 transparent;
    }
  }
  @keyframes cell-flash-b {
    0%,
    35% {
      box-shadow:
        0 0 0 2px var(--accent),
        0 0 16px 4px var(--accent);
    }
    100% {
      box-shadow:
        0 0 0 2px var(--accent),
        0 0 0 0 transparent;
    }
  }
  .cell.gone,
  .grid.gone {
    display: none;
  }
  .empty {
    margin: auto;
    max-width: 34em;
    padding: 24px;
    color: var(--muted);
    text-align: center;
  }
</style>
