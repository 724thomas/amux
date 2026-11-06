// The conversation a chat took over from the terminal's Claude, as rows to
// draw above the chat's own turns (read from the session's transcript by
// src-tauri/src/handoff.rs). Static once loaded, so plain objects.
import type { SubEntry } from "../ipc";
import type { SubTool } from "./subagents";

export type HistoryRow = { kind: "user" | "text"; text: string } | { kind: "tool"; tool: SubTool };

export function historyRows(entries: SubEntry[]): HistoryRow[] {
  const rows: HistoryRow[] = [];
  const tools = new Map<string, SubTool>();
  for (const e of entries) {
    if (e.k === "prompt") rows.push({ kind: "user", text: e.text });
    else if (e.k === "text") rows.push({ kind: "text", text: e.text });
    else if (e.k === "tool") {
      const tool: SubTool = { id: e.id, name: e.name, input: e.input ?? {}, done: false, err: false, result: null };
      tools.set(e.id, tool);
      rows.push({ kind: "tool", tool });
    } else {
      const t = tools.get(e.id);
      if (t) {
        t.done = true;
        t.err = e.err;
        t.result = e.text;
      }
    }
  }
  return rows;
}
