// Subagents' own files panels: a subagent that changes a file gets a cell of
// its own in the files pop-out window, right after its conversation's cell,
// that types its changes (three times the first speed). The cell lives as
// long as the subagent's card: done, it stays until the person's next message
// or the card's ✕.
//
// Claude sends a subagent's calls whole (no streaming) on the chat's output,
// a background subagent's too, a few hundredths of a second before the file
// changes. The main window reads them into each chat's model (reducer.ts,
// `subs`) and is the one source of the cells: it sends the pop-out window
// what to show (`files:subs`), the whole of it when the window asks (it opens
// only after the first change it is opened for), then each change.
import { emit, listen } from "@tauri-apps/api/event";
import { untrack } from "svelte";
import { workRead, workRoot, type PaneId } from "../ipc";
import { chats } from "./chat.svelte";
import { popOut, popout } from "./popout.svelte";
import { subs as cards } from "./subagentColumn.svelte";
import type { ChatItem, ChatModel } from "./reducer";

export const SUBS_EVENT = "files:subs";
export const SUBS_ASK = "files:subs-ask";

/** A subagent's cell: its card's number and description once its card is up. */
export interface SubCellMeta {
  id: string;
  label: string;
  finished: boolean;
}

/** One file-changing call of a subagent; `input` only when it is new. */
export interface SubRowMsg {
  toolUseId: string;
  name: string;
  path: string | null;
  input?: Record<string, unknown>;
  done: boolean;
  isError: boolean;
  awaiting: boolean;
  before?: string | null;
  arrivedMs?: number;
}

export type SubsMsg =
  | { kind: "state"; pane: PaneId; cells: SubCellMeta[]; rows: Record<string, SubRowMsg[]> }
  | { kind: "cells"; pane: PaneId; cells: SubCellMeta[] }
  | { kind: "row"; pane: PaneId; sub: string; row: SubRowMsg };

type ToolItem = Extract<ChatItem, { kind: "tool" }>;

/** Agent calls whose card was up at some point, per chat. */
const carded = new Map<PaneId, Set<string>>();

function cellsOf(pane: PaneId, m: ChatModel): SubCellMeta[] {
  const list = cards.byPane[pane]?.agents ?? [];
  const had = carded.get(pane) ?? new Set<string>();
  carded.set(pane, had);
  for (const a of list) if (a.toolUseId) had.add(a.toolUseId);
  const out: SubCellMeta[] = [];
  // In the order they were sent off (their cards' order).
  for (const s of Object.values(m.subs)) {
    if (s.items.length === 0) continue;
    const card = list.find((a) => a.toolUseId === s.id);
    // Its card was up and is gone (✕, the next message, its Claude gone): the
    // cell goes with it, whatever the model thinks. The card learns that a
    // subagent is done from its meta file, the model from the task notice,
    // and the two need not agree at the moment the person sends a message.
    if (!card && had.has(s.id)) continue;
    const what = (card?.description ?? s.description ?? "").trim();
    const who = card ? `서브에이전트 #${card.no}` : "서브에이전트";
    out.push({ id: s.id, label: what ? `${who} ${what}` : who, finished: s.finished });
  }
  return out;
}

/** `whole`: a call the pop-out window has not had (its input too); `withBefore`:
 *  the file as it was before a Write, which can be large, goes only once. */
function rowMsg(row: ToolItem, whole: boolean, withBefore: boolean): SubRowMsg {
  const msg: SubRowMsg = {
    toolUseId: row.toolUseId,
    name: row.name,
    path: row.path,
    done: row.done,
    isError: row.isError,
    awaiting: row.awaiting,
  };
  if (withBefore && row.before !== undefined) msg.before = row.before;
  if (whole) {
    msg.input = $state.snapshot(row.input) as Record<string, unknown>;
    msg.arrivedMs = row.arrivedMs;
  }
  return msg;
}

const beforeState = (row: ToolItem) => (row.before === undefined ? "-" : row.before === null ? "0" : "1");
const sig = (row: ToolItem) => `${row.done}|${row.isError}|${row.awaiting}|${beforeState(row)}`;

/** What the pop-out window was last told, per chat. */
const sent = new Map<PaneId, { model: ChatModel; cells: string; rows: Map<string, string> }>();
/** Cells that already asked for the pop-out window (once each). */
const announced = new Map<PaneId, Set<string>>();

function send(msg: SubsMsg) {
  void emit(SUBS_EVENT, msg).catch(() => {});
}

function publish(pane: PaneId, m: ChatModel) {
  const cells = cellsOf(pane, m);
  const key = JSON.stringify(cells);
  const prev = sent.get(pane);
  if (!prev || prev.model !== m) {
    // A new conversation (or the window asked): all of it.
    const rows: Record<string, SubRowMsg[]> = {};
    const sigs = new Map<string, string>();
    for (const c of cells) {
      rows[c.id] = [];
      for (const row of m.subs[c.id].items) {
        if (row.kind !== "tool") continue;
        rows[c.id].push(rowMsg(row, true, true));
        sigs.set(row.toolUseId, sig(row));
      }
    }
    sent.set(pane, { model: m, cells: key, rows: sigs });
    announced.set(pane, new Set(cells.map((c) => c.id)));
    send({ kind: "state", pane, cells, rows });
    return;
  }
  if (key !== prev.cells) {
    prev.cells = key;
    send({ kind: "cells", pane, cells });
  }
  for (const c of cells) {
    for (const row of m.subs[c.id].items) {
      if (row.kind !== "tool") continue;
      const now = sig(row);
      const was = prev.rows.get(row.toolUseId);
      if (was === now) continue;
      prev.rows.set(row.toolUseId, now);
      const fresh = was === undefined;
      send({ kind: "row", pane, sub: c.id, row: rowMsg(row, fresh, fresh || was.slice(-1) !== now.slice(-1)) });
      if (was === undefined && row.name === "Write" && row.before === undefined) void readBefore(m, row);
    }
  }
  // A subagent's first change: its cell wants the pop-out window.
  const seen = announced.get(pane) ?? new Set<string>();
  announced.set(pane, seen);
  let fresh = false;
  for (const c of cells) {
    if (seen.has(c.id)) continue;
    seen.add(c.id);
    fresh = true;
  }
  if (fresh) wantCells(pane);
}

/** Open the pop-out window with this chat's cell (its subagents' cells come
 *  after it), unless the person put the panel back during this request. */
function wantCells(pane: PaneId) {
  const s = chats.byPane[pane];
  if (!s || s.holdSeq === s.model.userSeq) return;
  s.panel = "open";
  if (!popout.panes.includes(pane)) popOut(pane);
}

/** A subagent's Write: read the file before it lands, so that a cell that
 *  opens only afterwards can still type the change (a Write cannot be undone
 *  from its own input the way an Edit can). */
async function readBefore(m: ChatModel, row: ToolItem) {
  const path = row.path;
  if (!path) return;
  const content = typeof row.input.content === "string" ? row.input.content : null;
  try {
    const cut = path.lastIndexOf("/");
    const r = await workRoot(cut > 0 ? path.slice(0, cut) : "/");
    if (!path.startsWith(r.root + "/")) return;
    const f = await workRead(r.root, path.slice(r.root.length + 1));
    const text = f.kind === "missing" ? "" : f.kind === "text" && !f.truncated ? f.text : null;
    // Read after the write already (or unreadable): there is no "before".
    if (text === null || text === content) return;
    row.before = text;
    m.subsSeq++;
  } catch {
    // A folder that does not exist yet: a new file, the panel starts it empty.
  }
}

let started = false;

/** Start publishing. The main window calls it once (from initChats). */
export function initSubPanels() {
  if (started) return;
  started = true;
  $effect.root(() => {
    $effect(() => {
      for (const [pane, s] of Object.entries(chats.byPane)) {
        void s.model.subsSeq;
        for (const a of cards.byPane[pane]?.agents ?? []) {
          void a.no;
          void a.toolUseId;
          void a.description;
        }
        const m = s.model;
        untrack(() => publish(pane, m));
      }
      untrack(() => {
        for (const pane of [...sent.keys()]) {
          if (chats.byPane[pane]) continue;
          sent.delete(pane);
          announced.delete(pane);
          carded.delete(pane);
          send({ kind: "state", pane, cells: [], rows: {} });
        }
      });
    });
  });
  void listen(SUBS_ASK, () => {
    sent.clear();
    for (const [pane, s] of Object.entries(chats.byPane)) publish(pane, s.model);
  });
}
