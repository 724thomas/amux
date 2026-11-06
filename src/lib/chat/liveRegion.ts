// How a change is drawn while it is being made: a region of the file (whole
// lines) as old lines and new lines in order, and how much of it has been
// "typed" so far.
//
// Each line is one segment: `keep` (old and new agree), `del` (an old line
// going away), `add` (a new line) or `rest` (an old line the arriving text has
// not reached yet). A cursor sweeps through the region: unchanged lines cost
// nothing, so it passes them at once; going lines are struck through one by
// one; new lines are typed character by character. Behind the cursor the
// change is drawn; ahead of it the region still shows the old lines.

import { diffLines } from "./lineDiff";

export interface Seg {
  kind: "keep" | "del" | "add" | "rest";
  text: string;
}

/** Cost of a going line, in typed characters: the strike sweeps down. */
const DEL_COST = 8;

/** How far ahead an unmatched new line may find its old twin. */
function window(line: string): number {
  return line.trim().length <= 2 ? 2 : 40;
}

/**
 * Segments for `oldText` against the new text that has arrived so far
 * (`arrived` may end in the middle of a line). Greedy: a new line equal to
 * the next old line keeps it; equal to an old line a little further on, the
 * old lines in between go; otherwise it is new. A half-arrived line that
 * could still turn out to be the next old line waits.
 */
export function streamSegments(oldText: string, arrived: string, complete: boolean): Seg[] {
  const O = oldText.split("\n");
  const N = arrived.split("\n");
  const full = complete ? N : N.slice(0, -1);
  const partial = complete ? null : N[N.length - 1];
  const segs: Seg[] = [];
  let adds: Seg[] = [];
  let oi = 0;
  for (const line of full) {
    if (oi < O.length && O[oi] === line) {
      segs.push(...adds, { kind: "keep", text: O[oi] });
      adds = [];
      oi++;
      continue;
    }
    let j = -1;
    const w = window(line);
    for (let k = 1; k <= w && oi + k < O.length; k++) {
      if (O[oi + k] === line) {
        j = oi + k;
        break;
      }
    }
    if (j >= 0) {
      segs.push(...adds);
      adds = [];
      for (let d = oi; d < j; d++) segs.push({ kind: "del", text: O[d] });
      segs.push({ kind: "keep", text: O[j] });
      oi = j + 1;
      continue;
    }
    adds.push({ kind: "add", text: line });
  }
  if (partial) {
    let mayKeep = false;
    for (let k = 0; k <= window(partial) && oi + k < O.length; k++) {
      if (O[oi + k].startsWith(partial)) {
        mayKeep = true;
        break;
      }
    }
    if (!mayKeep) adds.push({ kind: "add", text: partial });
  }
  segs.push(...adds);
  for (let d = oi; d < O.length; d++) segs.push({ kind: complete ? "del" : "rest", text: O[d] });
  return segs;
}

/** Segments for a change that is already complete: an exact line diff,
 *  going lines before their replacements. */
export function diffSegments(oldText: string, newText: string): Seg[] {
  const O = oldText.split("\n");
  const N = newText.split("\n");
  const segs: Seg[] = [];
  let oi = 0;
  for (const h of diffLines(O, N)) {
    for (; oi < h.aStart; oi++) segs.push({ kind: "keep", text: O[oi] });
    for (let d = h.aStart; d < h.aEnd; d++) segs.push({ kind: "del", text: O[d] });
    for (let a = h.bStart; a < h.bEnd; a++) segs.push({ kind: "add", text: N[a] });
    oi = h.aEnd;
  }
  for (; oi < O.length; oi++) segs.push({ kind: "keep", text: O[oi] });
  return segs;
}

export function totalCost(segs: Seg[]): number {
  let c = 0;
  for (const s of segs) c += s.kind === "add" ? s.text.length + 1 : s.kind === "del" ? DEL_COST : 0;
  return c;
}

export interface Composed {
  /** The region's text as drawn now. */
  text: string;
  /** Going lines (struck through), as offsets in `text`. */
  del: { from: number; to: number }[];
  /** Typed new text, as offsets in `text`. */
  add: { from: number; to: number }[];
  /** Where the cursor is, or null once everything is drawn. */
  caret: number | null;
}

/**
 * The region with `budget` characters' worth of the change drawn. With
 * `dropDeleted`, going lines are left out (the change has landed).
 */
export function composeRegion(segs: Seg[], budget: number, dropDeleted = false): Composed {
  const lines: string[] = [];
  const del: { from: number; to: number }[] = [];
  const add: { from: number; to: number }[] = [];
  let pos = 0;
  let spent = 0;
  let caret: number | null = null;
  let cut = false;
  const push = (text: string, mark: "del" | "add" | null) => {
    if (lines.length) pos += 1; // the newline before this line
    if (mark === "del" && text.length) del.push({ from: pos, to: pos + text.length });
    else if (mark === "del") del.push({ from: pos, to: pos });
    if (mark === "add") add.push({ from: pos, to: pos + text.length });
    lines.push(text);
    pos += text.length;
  };
  for (const s of segs) {
    if (cut) {
      // Ahead of the cursor: the old lines, as they were.
      if (s.kind !== "add") push(s.text, null);
      continue;
    }
    if (s.kind === "keep") push(s.text, null);
    else if (s.kind === "rest") {
      if (caret === null) caret = pos + (lines.length ? 1 : 0);
      cut = true;
      push(s.text, null);
    } else if (s.kind === "del") {
      if (spent + DEL_COST <= budget) {
        spent += DEL_COST;
        if (!dropDeleted) push(s.text, "del");
      } else {
        caret = pos + (lines.length ? 1 : 0);
        cut = true;
        push(s.text, null);
      }
    } else {
      const cost = s.text.length + 1;
      if (spent + cost <= budget) {
        spent += cost;
        push(s.text, "add");
      } else {
        const shown = Math.max(0, Math.floor(budget - spent));
        if (shown > 0) {
          push(s.text.slice(0, shown), "add");
          caret = pos;
        } else caret = pos + (lines.length ? 1 : 0);
        cut = true;
      }
    }
  }
  // Empty going lines still need a visible mark: give them their newline.
  for (const d of del) if (d.to === d.from && d.to < pos) d.to = d.from + 1;
  return { text: lines.join("\n"), del, add: add.filter((a) => a.to > a.from), caret };
}
