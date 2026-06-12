// Mirror of the Rust engine's Snapshot. The engine is the source of truth;
// every mutation lands here via the coarse "state:snapshot" event.
import { listen } from "@tauri-apps/api/event";
import {
  getSnapshot,
  type PaneId,
  type PaneInfo,
  type Snapshot,
  type WorkspaceInfo,
} from "./ipc";

export const app = $state<{ snapshot: Snapshot | null }>({ snapshot: null });

let initialized = false;

export async function initState() {
  if (initialized) return;
  initialized = true;
  await listen<Snapshot>("state:snapshot", (event) => {
    app.snapshot = event.payload;
  });
  // The engine creates the initial workspace (avoids double-create when the
  // dev server forces a page reload mid-bootstrap).
  app.snapshot = await getSnapshot();
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
