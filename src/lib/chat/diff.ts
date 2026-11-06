// Diff data for the chat view's edit cards: Claude Code reports every applied
// edit as hunks in `tool_use_result.structuredPatch`, drawn by DiffHunks.svelte.

export interface Hunk {
  oldStart: number;
  oldLines: number;
  newStart: number;
  newLines: number;
  /** Each line keeps its one-character prefix: " " context, "-" removed, "+" added. */
  lines: string[];
}

export interface DiffRow {
  /** `gap` separates two hunks (lines in between are unchanged and not shown). */
  kind: "ctx" | "del" | "add" | "gap";
  old: number | null;
  new: number | null;
  text: string;
}

export function isHunkList(value: unknown): value is Hunk[] {
  return (
    Array.isArray(value) &&
    value.every(
      (h) =>
        h != null &&
        typeof h === "object" &&
        Array.isArray((h as Hunk).lines) &&
        typeof (h as Hunk).oldStart === "number" &&
        typeof (h as Hunk).newStart === "number",
    )
  );
}

/** Hunks as display rows, each with its old and new line number. */
export function hunkRows(hunks: Hunk[]): DiffRow[] {
  const rows: DiffRow[] = [];
  hunks.forEach((h, index) => {
    if (index > 0) rows.push({ kind: "gap", old: null, new: null, text: "" });
    let oldNo = h.oldStart;
    let newNo = h.newStart;
    for (const line of h.lines) {
      const sign = line[0];
      const text = line.slice(1);
      if (sign === "-") rows.push({ kind: "del", old: oldNo++, new: null, text });
      else if (sign === "+") rows.push({ kind: "add", old: null, new: newNo++, text });
      else if (sign === "\\") continue; // "\ No newline at end of file"
      else rows.push({ kind: "ctx", old: oldNo++, new: newNo++, text });
    }
  });
  return rows;
}

export function countChanges(hunks: Hunk[]): { added: number; removed: number } {
  let added = 0;
  let removed = 0;
  for (const h of hunks) {
    for (const line of h.lines) {
      if (line[0] === "+") added++;
      else if (line[0] === "-") removed++;
    }
  }
  return { added, removed };
}

/** A new file's whole content as one all-added hunk. */
export function addedHunk(content: string): Hunk {
  const lines = content.split("\n");
  if (lines.length > 1 && lines[lines.length - 1] === "") lines.pop();
  return { oldStart: 0, oldLines: 0, newStart: 1, newLines: lines.length, lines: lines.map((l) => "+" + l) };
}
