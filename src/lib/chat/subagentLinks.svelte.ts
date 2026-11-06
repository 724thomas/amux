// Which Claude Code session runs in each terminal pane, asked of the backend
// (src-tauri/src/subagents.rs, `claude_links`) every few seconds while any
// subagent column is mounted: one question for all panes, not one per pane.
import { claudeLinks, type ClaudeLink, type PaneId } from "../ipc";

export const subLinks = $state<{ byPane: Record<PaneId, ClaudeLink> }>({ byPane: {} });

/** The last terminal Claude seen in each pane, and when (not reactive): a chat
 *  opened after it ended picks its conversation up. */
const lastTerm = new Map<PaneId, { session: string; at: number }>();
export function lastTermOf(pane: PaneId): { session: string; at: number } | undefined {
  return lastTerm.get(pane);
}

let users = 0;
let timer: ReturnType<typeof setTimeout> | null = null;
let last = "{}";

async function tick() {
  timer = null;
  let busy = false;
  try {
    const next = (await claudeLinks()) ?? {};
    const now = Date.now();
    for (const [pane, link] of Object.entries(next)) lastTerm.set(pane, { session: link.session, at: now });
    const text = JSON.stringify(next);
    // Only a real change reaches the columns (no wake-up every few seconds).
    if (text !== last) {
      last = text;
      subLinks.byPane = next;
    }
    busy = Object.values(next).some((l) => l.busy);
  } catch {
    // An older backend without the command: terminal panes stay unlinked.
  }
  // Slower while the window is hidden (minimized, another workspace).
  if (users > 0) timer = setTimeout(tick, document.hidden ? 6000 : busy ? 1500 : 3000);
}

/** A column mounted: keep the links fresh. Returns the matching release. */
export function useLinks(): () => void {
  users++;
  if (users === 1 && !timer) timer = setTimeout(tick, 0);
  return () => {
    users--;
    if (users === 0 && timer) {
      clearTimeout(timer);
      timer = null;
    }
  };
}
