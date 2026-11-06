// Which terminal panes show the files panel beside them (CLI mode). Kept out
// of PaneView, so a pane that is re-created when the layout changes keeps it.
import type { PaneId } from "../ipc";

export const termFiles = $state<{ open: Record<PaneId, boolean> }>({ open: {} });

export function setTermFiles(pane: PaneId, on: boolean) {
  if (on) termFiles.open[pane] = true;
  else delete termFiles.open[pane];
}
