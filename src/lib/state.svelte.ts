// Mirror of the Rust engine's Snapshot. The engine is the source of truth;
// every mutation lands here via the coarse "state:snapshot" event.
import { tick } from "svelte";
import { listen } from "@tauri-apps/api/event";
import {
  getSnapshot,
  type PaneId,
  type PaneInfo,
  type Snapshot,
  type WorkspaceInfo,
} from "./ipc";

export const app = $state<{ snapshot: Snapshot | null }>({ snapshot: null });

/** Panes currently flashing their attention ring (pane id → timeout id). */
export const rings = $state<{ active: Record<PaneId, boolean> }>({ active: {} });
const ringTimers = new Map<PaneId, ReturnType<typeof setTimeout>>();

// Each Terminal registers how to focus its xterm so that switching the
// active pane by ANY means (sidebar click, shortcut, CLI, split, close)
// lands the keyboard in the right terminal without an extra click.
const termFocus = new Map<PaneId, () => void>();

export function registerTermFocus(pane: PaneId, focus: () => void): () => void {
  termFocus.set(pane, focus);
  return () => {
    if (termFocus.get(pane) === focus) termFocus.delete(pane);
  };
}

/** Focus a pane's terminal (no-op while it isn't mounted/visible). */
export function focusTerm(pane: PaneId | null | undefined) {
  if (pane) termFocus.get(pane)?.();
}

function activeKey(snap: Snapshot | null): string {
  const ws = snap?.workspaces.find((w) => w.id === snap.active_workspace);
  return `${snap?.active_workspace ?? ""}:${ws?.active_pane ?? ""}`;
}

let initialized = false;

export async function initState() {
  if (initialized) return;
  initialized = true;
  await listen<Snapshot>("state:snapshot", (event) => {
    const before = activeKey(app.snapshot);
    app.snapshot = event.payload;
    if (activeKey(app.snapshot) !== before) {
      // After the DOM unhides the workspace, put the keyboard in it.
      void tick().then(() => focusTerm(activePane()));
    }
  });
  await listen<PaneId>("notify:ring", (event) => {
    const pane = event.payload;
    rings.active[pane] = true;
    clearTimeout(ringTimers.get(pane));
    ringTimers.set(
      pane,
      setTimeout(() => {
        delete rings.active[pane];
      }, 3000),
    );
  });
  // The engine creates the initial workspace (avoids double-create when the
  // dev server forces a page reload mid-bootstrap).
  app.snapshot = await getSnapshot();
  void tick().then(() => focusTerm(activePane()));
}

export function activeWorkspace(): WorkspaceInfo | null {
  const snap = app.snapshot;
  if (!snap?.active_workspace) return null;
  return snap.workspaces.find((w) => w.id === snap.active_workspace) ?? null;
}

export function activePane(): PaneId | null {
  return activeWorkspace()?.active_pane ?? null;
}

export function paneInfo(id: PaneId): PaneInfo | null {
  return app.snapshot?.panes.find((p) => p.id === id) ?? null;
}
