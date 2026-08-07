// In-window keyboard shortcuts (mouse-first UX — these are the auxiliary path).
// Used by both a window-level keydown listener and xterm's
// attachCustomKeyEventHandler, so shortcuts win over the terminal.
import {
  closeTab,
  closeWorkspace,
  focusPane,
  focusTab,
  focusWorkspace,
  newTab,
  splitPane,
  type LayoutNode,
  type PaneId,
} from "./ipc";
import {
  activePane,
  activeTab,
  activeWorkspace,
  app,
  broadcast,
  palette,
  dashboard,
  toggleComposerFocus,
  wsCreate,
} from "./state.svelte";
import { adjustFontSize, resetFontSize } from "./settings.svelte";

interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

function paneRects(node: LayoutNode, rect: Rect, out: Map<PaneId, Rect>) {
  if (node.type === "leaf") {
    out.set(node.pane, rect);
    return;
  }
  const r = node.ratio;
  if (node.axis === "horizontal") {
    paneRects(node.first, { ...rect, w: rect.w * r }, out);
    paneRects(node.second, { ...rect, x: rect.x + rect.w * r, w: rect.w * (1 - r) }, out);
  } else {
    paneRects(node.first, { ...rect, h: rect.h * r }, out);
    paneRects(node.second, { ...rect, y: rect.y + rect.h * r, h: rect.h * (1 - r) }, out);
  }
}

type Direction = "left" | "right" | "up" | "down";

/** Move focus to the neighbouring pane *within the current tab*. */
function navigate(direction: Direction) {
  const tab = activeTab();
  const current = activePane();
  if (!tab || !current) return;
  const rects = new Map<PaneId, Rect>();
  paneRects(tab.layout, { x: 0, y: 0, w: 1, h: 1 }, rects);
  const from = rects.get(current);
  if (!from) return;
  const fc = { x: from.x + from.w / 2, y: from.y + from.h / 2 };

  let best: { pane: PaneId; dist: number } | null = null;
  for (const [pane, r] of rects) {
    if (pane === current) continue;
    const c = { x: r.x + r.w / 2, y: r.y + r.h / 2 };
    const inDirection =
      direction === "left"
        ? c.x < fc.x - 1e-6
        : direction === "right"
          ? c.x > fc.x + 1e-6
          : direction === "up"
            ? c.y < fc.y - 1e-6
            : c.y > fc.y + 1e-6;
    if (!inDirection) continue;
    const dist = Math.hypot(c.x - fc.x, c.y - fc.y);
    if (!best || dist < best.dist) best = { pane, dist };
  }
  if (best) void focusPane(best.pane);
}

function cycleWorkspace(offset: number) {
  const snap = app.snapshot;
  if (!snap || snap.workspaces.length === 0) return;
  const index = snap.workspaces.findIndex((w) => w.id === snap.active_workspace);
  const next = (index + offset + snap.workspaces.length) % snap.workspaces.length;
  void focusWorkspace(snap.workspaces[next].id);
}

/** Jump to the Nth workspace by its sidebar position (0-based). Resolved live,
 *  so the index always points at whatever currently sits in that slot — close
 *  the 1st workspace and Ctrl+Shift+1 falls through to the new top one. */
function focusWorkspaceByIndex(index: number) {
  const ws = app.snapshot?.workspaces[index];
  if (ws) void focusWorkspace(ws.id);
}

/** Next / previous tab within the workspace on screen, wrapping around. */
function cycleTab(offset: number) {
  const ws = activeWorkspace();
  if (!ws || ws.tabs.length === 0) return;
  const index = ws.tabs.findIndex((t) => t.id === ws.active_tab);
  const next = (index + offset + ws.tabs.length) % ws.tabs.length;
  void focusTab(ws.tabs[next].id);
}

/** Jump to the Nth tab by its position in the tab bar (0-based). */
function focusTabByIndex(index: number) {
  const tab = activeWorkspace()?.tabs[index];
  if (tab) void focusTab(tab.id);
}

/**
 * Returns true when the event was consumed as an app shortcut.
 *
 * One keydown reaches us twice while the keyboard is inside a terminal: xterm's
 * `attachCustomKeyEventHandler` sees it first, declines to handle it, and the
 * event then bubbles on to the window listener. Running an action on both
 * passes would open two tabs per Ctrl+T, so the first pass stamps the event and
 * the second one only repeats the "consumed" answer (the caller still needs it
 * to `preventDefault`).
 */
export function handleKey(e: KeyboardEvent): boolean {
  if (e.type !== "keydown") return false;
  const stamped = e as KeyboardEvent & { __amuxHandled?: boolean };
  if (stamped.__amuxHandled) return true;
  const consumed = dispatch(e);
  if (consumed) stamped.__amuxHandled = true;
  return consumed;
}

function dispatch(e: KeyboardEvent): boolean {
  // Dashboard overlay is modal: Esc closes it.
  if (dashboard.open && e.key === "Escape") {
    dashboard.open = false;
    return true;
  }

  // Ctrl+Tab / Ctrl+Shift+Tab → next / previous tab, browser-style.
  if (e.ctrlKey && !e.altKey && e.code === "Tab") {
    cycleTab(e.shiftKey ? -1 : 1);
    return true;
  }

  if (e.ctrlKey && e.shiftKey && !e.altKey) {
    // Ctrl+Shift+1..9 → jump to the Nth workspace by sidebar position.
    if (/^Digit[1-9]$/.test(e.code)) {
      focusWorkspaceByIndex(Number(e.code.slice(5)) - 1);
      return true;
    }
    switch (e.code) {
      case "KeyN":
        // Open the new-workspace title prompt (the Sidebar renders the input).
        wsCreate.open = true;
        return true;
      case "KeyW": {
        // Closes the whole workspace — every tab in it, every pane in those.
        const ws = activeWorkspace();
        if (ws) void closeWorkspace(ws.id).catch(() => {});
        return true;
      }
      case "KeyD": {
        const pane = activePane();
        if (pane) void splitPane(pane, "horizontal");
        return true;
      }
      case "KeyS": {
        const pane = activePane();
        if (pane) void splitPane(pane, "vertical");
        return true;
      }
      case "KeyB":
        broadcast.on = !broadcast.on;
        return true;
      case "KeyP":
        palette.open = true;
        return true;
      case "KeyE":
        // Hop between the pane's bottom composer and its terminal (turning the
        // composer on if it's hidden).
        toggleComposerFocus(activePane());
        return true;
      case "KeyA":
        dashboard.open = !dashboard.open;
        return true;
    }
  }

  if (e.altKey && !e.ctrlKey && !e.shiftKey) {
    // Alt+1..9 → jump to the Nth tab of the workspace on screen.
    if (/^Digit[1-9]$/.test(e.code)) {
      focusTabByIndex(Number(e.code.slice(5)) - 1);
      return true;
    }
    const dir = {
      ArrowLeft: "left",
      ArrowRight: "right",
      ArrowUp: "up",
      ArrowDown: "down",
    }[e.key] as Direction | undefined;
    if (dir) {
      navigate(dir);
      return true;
    }
  }

  if (e.ctrlKey && !e.shiftKey && !e.altKey) {
    // Ctrl+T / Ctrl+W — browser tab muscle memory. Both are keys a shell would
    // otherwise use (Ctrl+W deletes a word, Ctrl+T is fzf's file search), so
    // they are deliberately taken from the terminal here.
    if (e.code === "KeyT") {
      const ws = activeWorkspace();
      if (ws) void newTab(ws.id);
      return true;
    }
    if (e.code === "KeyW") {
      const tab = activeTab();
      if (tab) void closeTab(tab.id).catch(() => {});
      return true;
    }
    if (e.key === "PageUp") {
      cycleWorkspace(-1);
      return true;
    }
    if (e.key === "PageDown") {
      cycleWorkspace(1);
      return true;
    }
    // Font zoom, GNOME Terminal-style.
    if (e.code === "Equal" || e.key === "+") {
      adjustFontSize(1);
      return true;
    }
    if (e.code === "Minus") {
      adjustFontSize(-1);
      return true;
    }
    if (e.code === "Digit0") {
      resetFontSize();
      return true;
    }
  }

  return false;
}
