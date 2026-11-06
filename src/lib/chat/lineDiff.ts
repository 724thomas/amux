// Line diff for replaying a change made outside the Edit/Write tools (a sed,
// a script, a formatter): which lines of the old text became which lines of
// the new one. liveRegion.ts turns the result into lines to strike and type.

export interface LineHunk {
  /** Lines [aStart, aEnd) of the old text became lines [bStart, bEnd) of the new. */
  aStart: number;
  aEnd: number;
  bStart: number;
  bEnd: number;
}

/** Myers' O((N+M)D) diff over lines; null when the texts differ in more
 *  than `maxD` lines (the caller then treats the middle as one block). */
function myersOps(a: string[], b: string[], maxD: number): ("=" | "-" | "+")[] | null {
  const n = a.length;
  const m = b.length;
  const max = Math.min(n + m, maxD);
  const off = max + 1;
  const v = new Int32Array(2 * max + 3);
  const trace: Int32Array[] = [];
  let found = -1;
  outer: for (let d = 0; d <= max; d++) {
    trace.push(v.slice());
    for (let k = -d; k <= d; k += 2) {
      let x = k === -d || (k !== d && v[off + k - 1] < v[off + k + 1]) ? v[off + k + 1] : v[off + k - 1] + 1;
      let y = x - k;
      while (x < n && y < m && a[x] === b[y]) {
        x++;
        y++;
      }
      v[off + k] = x;
      if (x >= n && y >= m) {
        found = d;
        break outer;
      }
    }
  }
  if (found < 0) return null;
  const ops: ("=" | "-" | "+")[] = [];
  let x = n;
  let y = m;
  for (let d = found; d > 0; d--) {
    const vd = trace[d];
    const k = x - y;
    const down = k === -d || (k !== d && vd[off + k - 1] < vd[off + k + 1]);
    const prevK = down ? k + 1 : k - 1;
    const prevX = vd[off + prevK];
    const prevY = prevX - prevK;
    while (x > prevX && y > prevY) {
      ops.push("=");
      x--;
      y--;
    }
    if (down) {
      ops.push("+");
      y--;
    } else {
      ops.push("-");
      x--;
    }
  }
  while (x > 0 && y > 0) {
    ops.push("=");
    x--;
    y--;
  }
  return ops.reverse();
}

/** Changed regions between two line lists, in order. */
export function diffLines(a: string[], b: string[], maxD = 1000): LineHunk[] {
  let start = 0;
  while (start < a.length && start < b.length && a[start] === b[start]) start++;
  let endA = a.length;
  let endB = b.length;
  while (endA > start && endB > start && a[endA - 1] === b[endB - 1]) {
    endA--;
    endB--;
  }
  if (start === endA && start === endB) return [];
  const whole = [{ aStart: start, aEnd: endA, bStart: start, bEnd: endB }];
  if (start === endA || start === endB) return whole;
  const ops = myersOps(a.slice(start, endA), b.slice(start, endB), maxD);
  if (!ops) return whole;
  const hunks: LineHunk[] = [];
  let ai = start;
  let bi = start;
  let i = 0;
  while (i < ops.length) {
    if (ops[i] === "=") {
      ai++;
      bi++;
      i++;
      continue;
    }
    const h = { aStart: ai, aEnd: ai, bStart: bi, bEnd: bi };
    while (i < ops.length && ops[i] !== "=") {
      if (ops[i] === "-") h.aEnd = ++ai;
      else h.bEnd = ++bi;
      i++;
    }
    hunks.push(h);
  }
  return hunks;
}
