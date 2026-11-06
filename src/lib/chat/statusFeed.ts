// A files-panel feed for a pane with no chat (a terminal running Claude Code
// or any other program), built from the pane's status. Claude Code's hooks
// report every finished tool call (`amux notify --kind progress` on
// PostToolUse), and each report reaches the page as a new snapshot; the owner
// calls `update` with the pane's status on every snapshot.
import type { PaneStatus } from "../ipc";
import type { FileFeed } from "./reducer";

export function emptyFeed(): FileFeed {
  return { editOrder: [], toolIds: {}, items: [], fileSeq: 0, status: "idle", userSeq: 0 };
}

/** Drive `feed` (a $state object) from status updates. */
export function statusFeed(feed: FileFeed) {
  // Look at most every 400 ms (snapshots come in bursts), and once after the last.
  let lastBump = 0;
  let trailing: ReturnType<typeof setTimeout> | null = null;
  function bump() {
    const wait = 400 - (Date.now() - lastBump);
    if (wait <= 0) {
      lastBump = Date.now();
      feed.fileSeq++;
    } else if (!trailing) {
      trailing = setTimeout(() => {
        trailing = null;
        lastBump = Date.now();
        feed.fileSeq++;
      }, wait);
    }
  }
  return {
    update(st: PaneStatus | undefined) {
      const next = st === "processing" ? "working" : st === "waiting" ? "waiting" : "idle";
      const was = feed.status;
      // A turn started: following comes back on, as at a new chat message.
      if (next !== "idle" && was === "idle") feed.userSeq++;
      feed.status = next;
      // While it works, any snapshot may follow a finished tool call. When the
      // turn ends, look once more: the engine drops a hook's report arriving
      // just after the turn's end, so the last change may have none of its own.
      if (next !== "idle" || was !== "idle") bump();
    },
    dispose() {
      if (trailing) clearTimeout(trailing);
    },
  };
}
