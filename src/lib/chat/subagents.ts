// The subagents of the Claude in a pane, as the column beside it shows them.
// The backend (src-tauri/src/subagents.rs) reads each subagent's transcript
// from the files Claude Code writes and sends compact entries; this folds them
// into a card's rows. Pure (no Svelte), so tests run it on real transcripts.
import type { AgentFile, SubEntry } from "../ipc";
import { toolSummary } from "./format";
import { diffLines } from "./lineDiff";

export interface SubTool {
  id: string;
  name: string;
  input: Record<string, unknown>;
  done: boolean;
  err: boolean;
  result: string | null;
}

/** A step refers to its tool by id: one object lives in one place only (a
 *  $state proxy reached by two paths would split in two). */
export type SubRow = { kind: "task" | "say" | "text"; text: string } | { kind: "tool"; id: string };

export interface SubAgent {
  id: string;
  /** Its card's number in the pane ("서브 1", "서브 2", ...). */
  no: number;
  type: string | null;
  description: string | null;
  createdMs: number;
  /** Its meta file says it is done. */
  finished: boolean;
  /** It will not finish: the person stopped it, or its Claude went away
   *  before it finished. */
  stopped: boolean;
  rows: SubRow[];
  tools: Record<string, SubTool>;
  /** Its final report (the hand-back message), once it gave one. */
  report: string | null;
  /** Read position in its transcript, and the transcript's size. */
  next: number;
  size: number;
  modifiedMs: number;
}

/** The tool a subagent hands its report back with: shown as the report. */
const HANDBACK = "SubagentHandback";

export function newAgent(f: AgentFile, no: number): SubAgent {
  return {
    id: f.id,
    no,
    type: f.agent_type,
    description: f.description,
    createdMs: f.created_ms,
    finished: f.finished,
    stopped: !f.finished && f.stopped,
    rows: [],
    tools: {},
    report: null,
    next: 0,
    size: f.size,
    modifiedMs: f.modified_ms,
  };
}

/** Its transcript was rewritten: read it again from the start. */
export function clearAgent(a: SubAgent) {
  a.rows = [];
  a.tools = {};
  a.report = null;
  a.next = 0;
}

export function applyEntries(a: SubAgent, items: SubEntry[]) {
  for (const it of items) {
    switch (it.k) {
      case "prompt":
        // The first message is its task; later ones were sent to it.
        a.rows.push({ kind: a.rows.length === 0 ? "task" : "say", text: it.text });
        break;
      case "text":
        a.rows.push({ kind: "text", text: it.text });
        break;
      case "tool": {
        a.tools[it.id] = { id: it.id, name: it.name, input: it.input ?? {}, done: false, err: false, result: null };
        if (it.name === HANDBACK) {
          const msg = (it.input ?? {}).message;
          if (typeof msg === "string" && msg.trim()) a.report = msg.trim();
        } else a.rows.push({ kind: "tool", id: it.id });
        break;
      }
      case "result": {
        const t = a.tools[it.id];
        if (t) {
          t.done = true;
          t.err = it.err;
          t.result = it.text;
        }
        break;
      }
    }
  }
}

export function running(a: SubAgent): boolean {
  return !a.finished && !a.stopped;
}

/** One step as a line: the tool and what it works on. */
export function stepLabel(t: SubTool, base: string | null): string {
  const s = toolSummary(t.name, t.input, base);
  return s ? `${t.name} ${s}` : t.name;
}

/** What it is doing right now: its latest step, while that is not done. */
export function activity(a: SubAgent, base: string | null): string | null {
  for (let i = a.rows.length - 1; i >= 0; i--) {
    const r = a.rows[i];
    if (r.kind !== "tool") continue;
    const t = a.tools[r.id];
    return t && !t.done ? stepLabel(t, base) : null;
  }
  return null;
}

export interface DiffLine {
  kind: "add" | "del" | "ctx" | "gap";
  text: string;
}

const str = (v: unknown) => (typeof v === "string" ? v : "");

/** A text's lines; the newline that ends the last one starts no line. */
function linesOf(text: string): string[] {
  const out = text.split("\n");
  if (out.length > 1 && out[out.length - 1] === "") out.pop();
  return out;
}

/** old -> new as the changed lines with `context` lines around each change.
 *  No line numbers: an Edit holds a piece of the file, not where it sits. */
export function changeLines(oldText: string, newText: string, context = 1): DiffLine[] {
  const a = linesOf(oldText);
  const b = linesOf(newText);
  const hunks = diffLines(a, b);
  const out: DiffLine[] = [];
  let i = 0;
  while (i < hunks.length) {
    // Hunks whose contexts touch print as one group.
    let j = i;
    while (j + 1 < hunks.length && hunks[j + 1].aStart - hunks[j].aEnd <= 2 * context) j++;
    const from = Math.max(0, hunks[i].aStart - context);
    const to = Math.min(a.length, hunks[j].aEnd + context);
    if (out.length) out.push({ kind: "gap", text: "" });
    let cur = from;
    for (let k = i; k <= j; k++) {
      const h = hunks[k];
      for (; cur < h.aStart; cur++) out.push({ kind: "ctx", text: a[cur] });
      for (let x = h.aStart; x < h.aEnd; x++) out.push({ kind: "del", text: a[x] });
      for (let y = h.bStart; y < h.bEnd; y++) out.push({ kind: "add", text: b[y] });
      cur = h.aEnd;
    }
    for (; cur < to; cur++) out.push({ kind: "ctx", text: a[cur] });
    i = j + 1;
  }
  return out;
}

/** The change a file step made, if it is one (Edit, MultiEdit, Write). */
export function stepDiff(t: SubTool): DiffLine[] | null {
  if (t.name === "Edit") return changeLines(str(t.input.old_string), str(t.input.new_string));
  if (t.name === "Write") return linesOf(str(t.input.content)).map((text) => ({ kind: "add", text }));
  if (t.name === "MultiEdit" && Array.isArray(t.input.edits)) {
    const out: DiffLine[] = [];
    for (const e of t.input.edits as Record<string, unknown>[]) {
      if (out.length) out.push({ kind: "gap", text: "" });
      out.push(...changeLines(str(e?.old_string), str(e?.new_string)));
    }
    return out;
  }
  return null;
}
