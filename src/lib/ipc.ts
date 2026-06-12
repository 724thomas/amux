// Typed wrappers around Tauri commands. All engine mutations go through here.
import { invoke, Channel } from "@tauri-apps/api/core";

export type PaneId = string;
export type WorkspaceId = string;
export type SplitAxis = "horizontal" | "vertical";

export interface PaneMeta {
  cwd: string | null;
  git_branch: string | null;
  listening_ports: number[];
  title: string | null;
  kitty_keyboard: boolean;
}

export interface PaneNotification {
  kind: "attention" | "done" | "progress" | "bell" | "idle";
  title: string | null;
  body: string | null;
}

export type PaneStatus = "none" | "processing" | "processed" | "idle" | "waiting";

export interface PaneInfo {
  id: PaneId;
  workspace: WorkspaceId;
  name: string;
  meta: PaneMeta;
  notification: PaneNotification | null;
  status: PaneStatus;
  exited: boolean;
}

export type LayoutNode =
  | { type: "leaf"; pane: PaneId }
  | {
      type: "split";
      axis: SplitAxis;
      ratio: number;
      first: LayoutNode;
      second: LayoutNode;
    };

export interface WorkspaceInfo {
  id: WorkspaceId;
  name: string;
  layout: LayoutNode;
  active_pane: PaneId | null;
}

export interface NotificationEntry {
  pane: PaneId;
  pane_name: string;
  kind: PaneNotification["kind"];
  title: string | null;
  body: string | null;
  at_ms: number;
}

export interface Snapshot {
  workspaces: WorkspaceInfo[];
  panes: PaneInfo[];
  active_workspace: WorkspaceId | null;
  notifications: NotificationEntry[];
}

export const getSnapshot = () => invoke<Snapshot>("get_snapshot");

// -- workspaces ---------------------------------------------------------------

export const createWorkspace = (name?: string, cols = 80, rows = 24) =>
  invoke<WorkspaceId>("create_workspace", { name: name ?? null, cols, rows });

export const closeWorkspace = (workspace: WorkspaceId) =>
  invoke<void>("close_workspace", { workspace });

export const focusWorkspace = (workspace: WorkspaceId) =>
  invoke<void>("focus_workspace", { workspace });

export const renameWorkspace = (workspace: WorkspaceId, name: string) =>
  invoke<void>("rename_workspace", { workspace, name });

export const moveWorkspace = (workspace: WorkspaceId, index: number) =>
  invoke<void>("move_workspace", { workspace, index });

export const setRatio = (workspace: WorkspaceId, path: boolean[], ratio: number) =>
  invoke<void>("set_ratio", { workspace, path, ratio });

// -- panes ----------------------------------------------------------------------

export const splitPane = (pane: PaneId, axis: SplitAxis, cols = 80, rows = 24) =>
  invoke<PaneId>("split_pane", { pane, axis, cols, rows });

export const focusPane = (pane: PaneId) => invoke<void>("focus_pane", { pane });

export const renamePane = (pane: PaneId, name: string) =>
  invoke<void>("rename_pane", { pane, name });

export const movePane = (pane: PaneId, target: PaneId, axis: SplitAxis, before: boolean) =>
  invoke<void>("move_pane", { pane, target, axis, before });

export const clearNotificationHistory = () =>
  invoke<void>("clear_notification_history");

export const writePane = (pane: PaneId, data: string) =>
  invoke<void>("write_pane", { pane, data });

export const resizePane = (pane: PaneId, cols: number, rows: number) =>
  invoke<void>("resize_pane", { pane, cols, rows });

export const closePane = (pane: PaneId) => invoke<void>("close_pane", { pane });

/** Subscribe to a pane's raw output bytes. Returns the channel (keep it alive). */
export function subscribePane(
  pane: PaneId,
  onData: (chunk: Uint8Array) => void,
): Channel<ArrayBuffer> {
  const channel = new Channel<ArrayBuffer>();
  channel.onmessage = (buf) => onData(new Uint8Array(buf));
  void invoke("pane_subscribe", { pane, channel });
  return channel;
}
