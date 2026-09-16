// Mirror of the Rust engine's Snapshot. The engine is the source of truth;
// every mutation lands here via the coarse "state:snapshot" event.
import { tick } from "svelte";
import { listen } from "@tauri-apps/api/event";
import {
  getSnapshot,
  restoreSession,
  type ResumeMode,
  savedSession,
  type LayoutNode,
  type PaneId,
  type PaneInfo,
  type SessionSummary,
  type Snapshot,
  type TabInfo,
  type WorkspaceId,
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

// Same idea for the bottom-pinned prompt composer: each Terminal registers a
// closure that moves the keyboard between its composer and its xterm, so
// Ctrl+Shift+E can hop back and forth without the shortcut knowing anything
// about the pane's internals.
const composerFocus = new Map<PaneId, () => void>();

export function registerComposerFocus(pane: PaneId, toggle: () => void): () => void {
  composerFocus.set(pane, toggle);
  return () => {
    if (composerFocus.get(pane) === toggle) composerFocus.delete(pane);
  };
}

/** Hop the keyboard between a pane's composer and its terminal. */
export function toggleComposerFocus(pane: PaneId | null | undefined) {
  if (pane) composerFocus.get(pane)?.();
}

function activeKey(snap: Snapshot | null): string {
  const ws = snap?.workspaces.find((w) => w.id === snap.active_workspace);
  const tab = ws?.tabs.find((t) => t.id === ws.active_tab);
  // Switching tabs changes what is on screen just as much as switching
  // workspaces, so the tab belongs in the key that re-triggers focus.
  return `${snap?.active_workspace ?? ""}:${ws?.active_tab ?? ""}:${tab?.active_pane ?? ""}`;
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
  // What the previous run left behind, if anything — read once, since the
  // engine's autosave overwrites the file as soon as a workspace exists.
  restoreOffer.summary = await savedSession();
}

// --- Previous-session restore ------------------------------------------------
// amux cannot bring back the *processes* it was running, but it does keep the
// arrangement: workspace names, tab names, the split shape of each tab and the
// directory every pane was working in. When the app goes away unexpectedly that
// shape is still on disk, and this offer puts all of it back in one click.
// `dismissed` only hides the offer for the rest of this run; the file is left
// alone, so a later launch can still offer it.
export const restoreOffer = $state<{
  summary: SessionSummary | null;
  dismissed: boolean;
  busy: boolean;
  /** Applied to every Claude pane the restore brings back. Both default to
   *  "leave it as it was": no `--effort` flag, and the resume menu answered by
   *  the person, because the other choices spend usage limits without a click. */
  effort: string | null;
  mode: ResumeMode;
}>({ summary: null, dismissed: false, busy: false, effort: null, mode: "ask" });

/** Whether the restore card / palette entry should be on offer right now. */
export function canRestoreSession(): boolean {
  return restoreOffer.summary !== null && !restoreOffer.dismissed;
}

/** Rebuild everything the previous session held. Restoring is one-shot: the
 *  engine forgets the saved session afterwards so a second click cannot
 *  duplicate every workspace. */
export async function restorePreviousSession() {
  if (!restoreOffer.summary || restoreOffer.busy) return;
  restoreOffer.busy = true;
  try {
    await restoreSession({ effort: restoreOffer.effort, mode: restoreOffer.mode });
    restoreOffer.summary = null;
  } catch (e) {
    console.error("지난 세션 복구 실패", e);
  } finally {
    restoreOffer.busy = false;
  }
}

export function activeWorkspace(): WorkspaceInfo | null {
  const snap = app.snapshot;
  if (!snap?.active_workspace) return null;
  return snap.workspaces.find((w) => w.id === snap.active_workspace) ?? null;
}

/** The tab currently on screen in the active workspace. */
export function activeTab(): TabInfo | null {
  const ws = activeWorkspace();
  if (!ws) return null;
  return ws.tabs.find((t) => t.id === ws.active_tab) ?? null;
}

export function activePane(): PaneId | null {
  return activeTab()?.active_pane ?? null;
}

export function paneInfo(id: PaneId): PaneInfo | null {
  return app.snapshot?.panes.find((p) => p.id === id) ?? null;
}

/** Every pane in a split tree, left-to-right / top-to-bottom. */
export function layoutPanes(node: LayoutNode): PaneId[] {
  return node.type === "leaf"
    ? [node.pane]
    : [...layoutPanes(node.first), ...layoutPanes(node.second)];
}

/** The tab a pane lives in — where its user-visible name comes from. */
export function tabOfPane(id: PaneId): TabInfo | null {
  const pane = paneInfo(id);
  if (!pane) return null;
  const ws = app.snapshot?.workspaces.find((w) => w.id === pane.workspace);
  return ws?.tabs.find((t) => t.id === pane.tab) ?? null;
}

/** Every live pane in a tab (the reach of a broadcast, and of the ordinals). */
export function tabPanes(tab: TabInfo): PaneId[] {
  return layoutPanes(tab.layout);
}

/**
 * Attention order, shared by every list that ranks panes: 🟡 waiting first,
 * then 🟢 processed, 🔴 processing, 🔵 idle, and DONE last — the user already
 * pinned that one as reviewed, so it is never asking for anything.
 */
export const statusRank = (s: string): number =>
  s === "waiting" ? 0 : s === "processed" ? 1 : s === "processing" ? 2 : s === "idle" ? 3 : 4;

/** The most attention-hungry status among a tab's live panes — the tab's chip. */
export function tabStatus(tab: TabInfo): PaneInfo["status"] | null {
  const ids = new Set(tabPanes(tab));
  let best: PaneInfo["status"] | null = null;
  for (const p of app.snapshot?.panes ?? []) {
    if (p.exited || !ids.has(p.id)) continue;
    if (best === null || statusRank(p.status) < statusRank(best)) best = p.status;
  }
  return best;
}

/** True when any pane in the tab is still holding an unseen notification. */
export function tabHasBadge(tab: TabInfo): boolean {
  const ids = new Set(tabPanes(tab));
  return (app.snapshot?.panes ?? []).some((p) => ids.has(p.id) && p.notification !== null);
}

/**
 * What to call a pane in per-pane lists (dashboard, 손길 필요 목록, palette).
 * Panes have no name; the tab does. A tab holding a single pane lends its name
 * as-is — split it and each pane gets an ordinal so two rows can't look alike.
 */
export function paneLabel(id: PaneId): string {
  const tab = tabOfPane(id);
  if (!tab) return "터미널";
  const panes = tabPanes(tab);
  if (panes.length <= 1) return tab.name;
  const index = panes.indexOf(id);
  return index < 0 ? tab.name : `${tab.name} #${index + 1}`;
}

// --- Broadcast (synchronize-panes) -----------------------------------------
// When on, keyboard input to the focused pane is mirrored to every other live
// pane in the SAME TAB — "type once, command every agent you can see". Scoped
// to the tab on purpose: input can never reach a terminal that is off screen.
// Transient and default-off: a powerful mode you opt into per session
// (Ctrl+Shift+B); it never persists across restarts, so it can't surprise you
// on a fresh launch.
export const broadcast = $state<{ on: boolean }>({ on: false });

// Command Palette (Ctrl+Shift+P) open/closed. Transient.
export const palette = $state<{ open: boolean }>({ open: false });

// New-workspace title prompt: creating a workspace (via the sidebar "+" button
// or Ctrl+Shift+N) always opens an inline title input first — no workspace is
// created until the user confirms. Shared so both entry points drive the one
// input the Sidebar renders. Transient.
export const wsCreate = $state<{ open: boolean }>({ open: false });

// New-tab title prompt, the same idea one level down: Ctrl+T, the tab bar "+"
// and the palette all just name the workspace to open a tab in, and the tab bar
// renders one inline input at the end of that workspace's tabs. Nothing is
// created until the user confirms; a blank name falls back to the engine's
// auto-name (`탭 N`). Transient.
export const tabCreate = $state<{ workspace: WorkspaceId | null }>({ workspace: null });

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
      name: paneLabel(p.id),
      workspace: ws?.name ?? "",
      workspaceId: p.workspace,
      status: p.status,
      since: statusSince.get(p.id)?.since ?? clock.now,
      branch: p.meta.git_branch ?? null,
      cwd: p.meta.cwd ?? null,
    });
  }
  // Attention order first, then oldest-in-status first.
  out.sort((a, b) => statusRank(a.status) - statusRank(b.status) || a.since - b.since);
  return out;
}

/** Aggregate live-agent counts by status, for the dashboard header / strip. */
export function statusCounts(): {
  processing: number;
  waiting: number;
  processed: number;
  idle: number;
  done: number;
  total: number;
} {
  const c = { processing: 0, waiting: 0, processed: 0, idle: 0, done: 0, total: 0 };
  const snap = app.snapshot;
  if (!snap) return c;
  for (const p of snap.panes) {
    if (p.exited) continue;
    c.total++;
    if (p.status === "processing") c.processing++;
    else if (p.status === "waiting") c.waiting++;
    else if (p.status === "processed") c.processed++;
    else if (p.status === "idle") c.idle++;
    else if (p.status === "done") c.done++;
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
        name: paneLabel(p.id),
        workspace: wsName,
        status: p.status,
        since: statusSince.get(p.id)?.since ?? clock.now,
      });
    }
  }
  out.sort((a, b) => statusRank(a.status) - statusRank(b.status) || a.since - b.since);
  return out;
}

/** Live, non-exited panes sharing `origin`'s tab (excluding `origin` itself). */
export function broadcastTargets(origin: PaneId): PaneId[] {
  const snap = app.snapshot;
  const tab = paneInfo(origin)?.tab;
  if (!snap || !tab) return [];
  return snap.panes
    .filter((p) => p.tab === tab && p.id !== origin && !p.exited)
    .map((p) => p.id);
}

/** How many panes a broadcast reaches: every live pane in the visible tab. */
export function activeTabPaneCount(): number {
  const snap = app.snapshot;
  const tab = activeTab();
  if (!snap || !tab) return 0;
  return snap.panes.filter((p) => p.tab === tab.id && !p.exited).length;
}
