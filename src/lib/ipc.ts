// Typed wrappers around Tauri commands. All engine mutations go through here.
import { invoke, Channel } from "@tauri-apps/api/core";

export type PaneId = string;

export interface PaneMeta {
  cwd: string | null;
  git_branch: string | null;
  listening_ports: number[];
  title: string | null;
}

export interface PaneInfo {
  id: PaneId;
  workspace: string;
  meta: PaneMeta;
  notification: { kind: string; title: string | null; body: string | null } | null;
  exited: boolean;
}

export type LayoutNode =
  | { type: "leaf"; pane: PaneId }
  | {
      type: "split";
      axis: "horizontal" | "vertical";
      ratio: number;
      first: LayoutNode;
      second: LayoutNode;
    };

export interface WorkspaceInfo {
  id: string;
  name: string;
  layout: LayoutNode;
  active_pane: PaneId | null;
}

export interface Snapshot {
  workspaces: WorkspaceInfo[];
  panes: PaneInfo[];
  active_workspace: string | null;
}

export const getSnapshot = () => invoke<Snapshot>("get_snapshot");

export const createPane = (cols: number, rows: number) =>
  invoke<PaneId>("create_pane", { cols, rows });

export const writePane = (pane: PaneId, data: string) =>
  invoke<void>("write_pane", { pane, data });

export const resizePane = (pane: PaneId, cols: number, rows: number) =>
  invoke<void>("resize_pane", { pane, cols, rows });

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
