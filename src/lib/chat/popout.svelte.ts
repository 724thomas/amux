// Which panes' files panels are in the pop-out window, as the main window
// sees it. The list lives in the app (src-tauri/src/popout.rs), so both
// windows share it; it arrives here as `files:state` events.
import { listen } from "@tauri-apps/api/event";
import { filesLook, filesPop, filesState, type Look, type PaneId, type PopoutState } from "../ipc";
import { settings } from "../settings.svelte";

export const popout = $state<PopoutState>({ panes: [], look: null });

function take(s: PopoutState) {
  popout.panes = s.panes;
  popout.look = s.look;
}

let started: Promise<void> | null = null;

/** Start following the list. Idempotent; the main window calls it once at start. */
export function initPopout(): Promise<void> {
  // Listen first, then ask: a change landing in between is not lost.
  started ??= listen<PopoutState>("files:state", (e) => take(e.payload))
    .then(() => filesState())
    .then(take)
    .catch((e) => console.warn("[amux] files pop-out state", e));
  return started;
}

export function isPopped(pane: PaneId): boolean {
  return popout.panes.includes(pane);
}

export function lookNow(): Look {
  return { theme: settings.theme, fontSize: settings.fontSize };
}

/** Move this pane's files panel into the pop-out window (opening it). */
export function popOut(pane: PaneId) {
  void filesPop(pane, true, lookNow())
    .then(take)
    .catch((e) => console.warn("[amux] pop out", e));
}

/** Put this pane's files panel back in its pane. */
export function dockBack(pane: PaneId) {
  void filesPop(pane, false, null)
    .then(take)
    .catch((e) => console.warn("[amux] dock back", e));
}

/** The main window's theme or font size changed: tell an open pop-out. */
export function sendLook() {
  if (popout.panes.length === 0) return;
  void filesLook(lookNow()).catch(() => {});
}
