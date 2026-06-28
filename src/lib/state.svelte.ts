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

// Who-Needs-Me: per-pane time-in-current-status, derived by diffing snapshots
// on the frontend (no engine change). `clock` ticks once a second so the
// "how long" timers update live.
export const clock = $state<{ now: number }>({ now: Date.now() });
const statusSince = new Map<PaneId, { status: string; since: number }>();
function trackStatus(snap: Snapshot) {
  const now = Date.now();
  const seen = new Set<PaneId>();
  for (const p of snap.panes) {
    seen.add(p.id);
    const prev = statusSince.get(p.id);
    if (!prev || prev.status !== p.status) statusSince.set(p.id, { status: p.status, since: now });
  }
  for (const id of [...statusSince.keys()]) if (!seen.has(id)) statusSince.delete(id);
}

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
    trackStatus(event.payload);
    if (activeKey(app.snapshot) !== before) {
      // After the DOM unhides the workspace, put the keyboard in it.
      void tick().then(() => focusTerm(activePane()));
    }
  });
  // Live 1s clock so the Who-Needs-Me timers tick.
  setInterval(() => (clock.now = Date.now()), 1000);
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
  if (app.snapshot) trackStatus(app.snapshot);
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

// --- Broadcast (synchronize-panes) -----------------------------------------
// When on, keyboard input to the focused pane is mirrored to every other live
// pane in the ACTIVE workspace — "type once, command every agent". Transient
// and default-off: a powerful mode you opt into per session (Ctrl+Shift+B); it
// never persists across restarts, so it can't surprise you on a fresh launch.
export const broadcast = $state<{ on: boolean }>({ on: false });

// Command Palette (Ctrl+Shift+P) open/closed. Transient.
export const palette = $state<{ open: boolean }>({ open: false });

// --- Dashboard (Mission Control) -------------------------------------------
// A JARVIS-style full-screen overlay (Ctrl+Shift+A) showing every live agent
// across all workspaces at a glance. Transient.
export const dashboard = $state<{ open: boolean }>({ open: false });

export interface AgentTile {
  pane: PaneId;
  name: string;
  workspace: string;
  workspaceId: string;
  status: string;
  since: number;
  branch: string | null;
  cwd: string | null;
}

/** Every live agent (pane) across all workspaces, for the dashboard grid. */
export function dashboardAgents(): AgentTile[] {
  const snap = app.snapshot;
  if (!snap) return [];
  const out: AgentTile[] = [];
  for (const p of snap.panes) {
    if (p.exited) continue;
    const ws = snap.workspaces.find((w) => w.id === p.workspace);
    out.push({
      pane: p.id,
      name: p.name,
      workspace: ws?.name ?? "",
      workspaceId: p.workspace,
      status: p.status,
      since: statusSince.get(p.id)?.since ?? clock.now,
      branch: p.meta.git_branch ?? null,
      cwd: p.meta.cwd ?? null,
    });
  }
  // waiting → processed → processing → idle, then oldest-in-status first.
  const rank = (s: string) =>
    s === "waiting" ? 0 : s === "processed" ? 1 : s === "processing" ? 2 : 3;
  out.sort((a, b) => rank(a.status) - rank(b.status) || a.since - b.since);
  return out;
}

/** Aggregate live-agent counts by status, for the dashboard header / strip. */
export function statusCounts(): {
  processing: number;
  waiting: number;
  processed: number;
  idle: number;
  total: number;
} {
  const c = { processing: 0, waiting: 0, processed: 0, idle: 0, total: 0 };
  const snap = app.snapshot;
  if (!snap) return c;
  for (const p of snap.panes) {
    if (p.exited) continue;
    c.total++;
    if (p.status === "processing") c.processing++;
    else if (p.status === "waiting") c.waiting++;
    else if (p.status === "processed") c.processed++;
    else if (p.status === "idle") c.idle++;
  }
  return c;
}

// --- Who-Needs-Me list ------------------------------------------------------
export interface AttentionItem {
  pane: PaneId;
  name: string;
  workspace: string;
  status: "waiting" | "processed";
  since: number;
}

/** Panes that want the user: 🟡 waiting first, then 🟢 processed; oldest first. */
export function attentionItems(): AttentionItem[] {
  const snap = app.snapshot;
  if (!snap) return [];
  const out: AttentionItem[] = [];
  for (const p of snap.panes) {
    if (p.exited) continue;
    if (p.status === "waiting" || p.status === "processed") {
      const wsName = snap.workspaces.find((w) => w.id === p.workspace)?.name ?? "";
      out.push({
        pane: p.id,
        name: p.name,
        workspace: wsName,
        status: p.status,
        since: statusSince.get(p.id)?.since ?? clock.now,
      });
    }
  }
  const rank = (s: string) => (s === "waiting" ? 0 : 1);
  out.sort((a, b) => rank(a.status) - rank(b.status) || a.since - b.since);
  return out;
}

/** Live, non-exited panes in the active workspace other than `origin`. */
export function broadcastTargets(origin: PaneId): PaneId[] {
  const snap = app.snapshot;
  const ws = snap?.active_workspace;
  if (!snap || !ws) return [];
  return snap.panes
    .filter((p) => p.workspace === ws && p.id !== origin && !p.exited)
    .map((p) => p.id);
}

/** How many panes a broadcast reaches (all live panes in the active workspace). */
export function activeWorkspacePaneCount(): number {
  const snap = app.snapshot;
  const ws = snap?.active_workspace;
  if (!snap || !ws) return 0;
  return snap.panes.filter((p) => p.workspace === ws && !p.exited).length;
}
