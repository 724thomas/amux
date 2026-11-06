// The subagent column of each pane: which subagents get a card, read from the
// Claude Code session on screen in that pane (a terminal's Claude or the
// chat's). The state lives here, not in the component, so it survives the pane
// view being rebuilt (a split or a move remounts it).
//
// Rules (what the person chose):
//   a subagent that starts gets a card at once, stacked under the others;
//   a finished card stays (done mark and report) until the person's next
//   message, then closes; a running one stays until it is done;
//   no cards: no column, the main part takes the whole pane again.
import { sessionPrompts, subagentRead, subagentsList, type PaneId } from "../ipc";
import { applyEntries, clearAgent, newAgent, running, type SubAgent } from "./subagents";

/** The Claude on screen in a pane, whose subagents the column shows. */
export interface Source {
  session: string;
  /** Its process is alive; a dead one's subagents cannot be running. */
  alive: boolean;
  /** When that process started (ms since the epoch), null when not known. */
  startedMs: number | null;
  /** A chat counts the person's messages itself; null: count them from the
   *  main transcript (a terminal's Claude). */
  prompts: number | null;
  /** Claude is working: look more often. */
  busy: boolean;
}

export interface PaneSubs {
  session: string | null;
  /** The first look at this session was made: what was there then and not
   *  running is not shown. */
  scanned: boolean;
  agents: SubAgent[];
  /** Agents not to show: earlier ones found at the first look, closed ones. */
  skip: Record<string, true>;
  /** Read position in the main transcript, for counting prompts. */
  promptAt: number | null;
  /** The chat's message counter when last looked at. */
  userSeq: number;
  /** The last card number given out. */
  seq: number;
  /** When the Claude went missing (its cards go after a short grace). */
  goneAt: number | null;
}

export const subs = $state<{ byPane: Record<PaneId, PaneSubs> }>({ byPane: {} });

/** One missed look at the links is not an exit. */
const GONE_GRACE = 4000;
/** Transcript reads per look, at most (the rest come next time). */
const READS = 6;
/** Agents first seen this long before their Claude started are older. */
const START_SLACK = 2000;

function blank(): PaneSubs {
  return { session: null, scanned: false, agents: [], skip: {}, promptAt: null, userSeq: 0, seq: 0, goneAt: null };
}

function reset(e: PaneSubs, session: string | null, userSeq: number) {
  e.session = session;
  e.scanned = false;
  e.agents = [];
  e.skip = {};
  e.promptAt = null;
  e.userSeq = userSeq;
  e.seq = 0;
  e.goneAt = null;
}

/** The person sent a message: finished and stopped cards close. */
function dismiss(e: PaneSubs) {
  const keep: SubAgent[] = [];
  for (const a of e.agents) {
    if (running(a)) keep.push(a);
    else e.skip[a.id] = true;
  }
  if (keep.length !== e.agents.length) e.agents = keep;
  if (keep.length === 0) e.seq = 0;
}

/** ✕ on a finished card. */
export function closeCard(pane: PaneId, id: string) {
  const e = subs.byPane[pane];
  if (!e) return;
  e.skip[id] = true;
  e.agents = e.agents.filter((a) => a.id !== id);
  if (e.agents.length === 0) e.seq = 0;
}

/** The pane is gone (closed). */
export function forgetPane(pane: PaneId) {
  delete subs.byPane[pane];
}

/** One look: new subagents, their new transcript lines, new prompts.
 *  Returns whether to look again soon (something is running). */
export async function poll(pane: PaneId, src: Source | null): Promise<boolean> {
  let e = subs.byPane[pane];
  if (!src) {
    if (!e?.session) return false;
    e.goneAt ??= Date.now();
    if (Date.now() - e.goneAt < GONE_GRACE) return true;
    // The Claude exited (or the pane switched to one without a session):
    // its cards go with it.
    reset(e, null, 0);
    return false;
  }
  if (!e) {
    subs.byPane[pane] = blank();
    e = subs.byPane[pane];
  }
  e.goneAt = null;
  if (e.session !== src.session) reset(e, src.session, src.prompts ?? 0);
  const session = src.session;
  const stale = () => subs.byPane[pane] !== e || e.session !== session;

  if (!src.alive) for (const a of e.agents) if (running(a)) a.stopped = true;

  // Did the person send a message since the last look?
  let prompted = false;
  if (src.prompts != null) {
    prompted = src.prompts > e.userSeq;
    e.userSeq = src.prompts;
  } else {
    const p = await sessionPrompts(session, e.promptAt);
    if (stale()) return true;
    prompted = p.count > 0 && e.promptAt != null;
    e.promptAt = p.next;
  }

  const list = await subagentsList(session);
  if (stale()) return true;
  // The cards already shown learn they are done first, so that a message
  // closes every card that is done by now (one that finished just before it
  // too); new subagents are taken in only after that, as the new turn may
  // have started them.
  for (const a of e.agents) {
    const f = list.find((x) => x.id === a.id);
    if (f?.finished) {
      a.finished = true;
      a.stopped = false;
    } else if (f?.stopped) a.stopped = true;
  }
  if (prompted) dismiss(e);
  const since = (src.startedMs ?? Date.now() - 60_000) - START_SLACK;
  let reads = 0;
  for (const f of list) {
    let a = e.agents.find((x) => x.id === f.id);
    if (!a) {
      if (e.skip[f.id]) continue;
      // At the first look only a subagent of this very run that is still
      // working gets a card; any later newcomer does (even one that already
      // finished between two looks).
      const show = e.scanned || (src.alive && !f.finished && !f.stopped && f.modified_ms >= since);
      if (!show) {
        e.skip[f.id] = true;
        continue;
      }
      e.agents.push(newAgent(f, ++e.seq));
      a = e.agents[e.agents.length - 1];
    }
    if (f.agent_type) a.type = f.agent_type;
    if (f.description) a.description = f.description;
    a.size = f.size;
    a.modifiedMs = f.modified_ms;
    if (f.finished) {
      a.finished = true;
      a.stopped = false;
    } else if (f.stopped) a.stopped = true;
    while (a.next !== f.size && reads < READS) {
      reads++;
      const c = await subagentRead(session, a.id, a.next);
      if (stale()) return true;
      if (c.reset) clearAgent(a);
      applyEntries(a, c.items);
      if (c.next === a.next) break; // only part of a line so far
      a.next = c.next;
    }
  }
  e.scanned = true;
  return src.busy || e.agents.some((a) => running(a) || a.next !== a.size);
}
