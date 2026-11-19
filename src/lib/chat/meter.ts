// The `[ctx:N%] / [5h, week]` meter in the chat header. Same sums, rounding,
// colors and wording as the terminal status line (the user's
// ~/.claude/statusline-command.sh, which Claude Code feeds `context_window`
// and which reads amux's usage file), so a pane reads the same in either mode.
//
// Plain TypeScript (no runes) so `bun` can check it without the app.

import type { ClaudeUsage } from "../ipc";

/** How full the context is, in whole percent: what the last reply read over
 *  the model's window, rounded and kept within 0 to 100 as Claude Code's
 *  status line does. Null while either is unknown. */
export function ctxPercent(tokens: number | null, window: number | null): number | null {
  if (tokens === null || !window || window <= 0) return null;
  return Math.min(100, Math.max(0, Math.round((tokens / window) * 100)));
}

/** The status line's colors: 80 and up red, 50 and up yellow, below green. */
export function tone(pct: number): "low" | "mid" | "high" {
  return pct >= 80 ? "high" : pct >= 50 ? "mid" : "low";
}

/** Time left until `resetsAt` (epoch seconds): "2d 13h 2m", "3h 43m" or
 *  "43m", the minutes rounded down. */
export function remaining(resetsAt: number, nowSec: number): string {
  const left = Math.max(0, resetsAt - nowSec);
  const d = Math.floor(left / 86400);
  const h = Math.floor((left % 86400) / 3600);
  const m = Math.floor((left % 3600) / 60);
  if (d > 0) return `${d}d ${h}h ${m}m`;
  if (h > 0) return `${h}h ${m}m`;
  return `${m}m`;
}

export interface LimitShown {
  /** "5시간" or "주간", for the tooltip. */
  label: string;
  pct: number;
  /** Time left until it resets; null for a window not used yet (no reset time). */
  when: string | null;
}

/** A file older than this means amux stopped polling: show nothing rather
 *  than an old number. */
const STALE_AFTER_SEC = 15 * 60;

/** The 5-hour and weekly windows to show, in that order. A window past its
 *  reset is left out until amux's next poll (20 s after the reset). */
export function limitsShown(usage: ClaudeUsage | null, nowSec: number): LimitShown[] {
  if (!usage || nowSec - usage.fetched_at > STALE_AFTER_SEC) return [];
  const out: LimitShown[] = [];
  const windows = [
    ["5시간", usage.five_hour],
    ["주간", usage.seven_day],
  ] as const;
  for (const [label, w] of windows) {
    if (!w) continue;
    if (w.resets_at !== null && w.resets_at <= nowSec) continue;
    out.push({
      label,
      pct: Math.round(w.used_percentage),
      when: w.resets_at !== null ? remaining(w.resets_at, nowSec) : null,
    });
  }
  return out;
}
