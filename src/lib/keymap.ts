// In-window keyboard shortcuts (mouse-first UX — these are the auxiliary path).
// Used by both a window-level keydown listener and xterm's
// attachCustomKeyEventHandler, so shortcuts win over the terminal.
import {
  closePane,
  createWorkspace,
  focusPane,
  focusWorkspace,
  splitPane,
  type LayoutNode,
  type PaneId,
} from "./ipc";
import { activePane, activeWorkspace, app } from "./state.svelte";
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

function navigate(direction: Direction) {
  const ws = activeWorkspace();
  const current = activePane();
  if (!ws || !current) return;
  const rects = new Map<PaneId, Rect>();
  paneRects(ws.layout, { x: 0, y: 0, w: 1, h: 1 }, rects);
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

/** Returns true when the event was consumed as an app shortcut. */
export function handleKey(e: KeyboardEvent): boolean {
  if (e.type !== "keydown") return false;

  if (e.ctrlKey && e.shiftKey && !e.altKey) {
    switch (e.code) {
      case "KeyT":
        void createWorkspace();
        return true;
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
      case "KeyW": {
        const pane = activePane();
        if (pane) void closePane(pane);
        return true;
      }
    }
  }

  if (e.altKey && !e.ctrlKey && !e.shiftKey) {
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
