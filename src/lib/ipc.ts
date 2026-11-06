// Typed wrappers around Tauri commands. All engine mutations go through here.
import { invoke, Channel } from "@tauri-apps/api/core";

export type PaneId = string;
export type WorkspaceId = string;
export type TabId = string;
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

/// `done` is the odd one out: the user pins it from the pane toolbar and no
/// automatic writer (hooks, bell, silence heuristic, focus) may overwrite it.
/// Being `done` IS the pin — there is no separate flag.
export type PaneStatus =
  | "processing"
  | "processed"
  | "idle"
  | "waiting"
  | "done";

/// A pane has no name of its own — the tab it belongs to is the named unit,
/// and every pane inside borrows that name (see `paneLabel` in state.svelte).
export interface PaneInfo {
  id: PaneId;
  workspace: WorkspaceId;
  tab: TabId;
  meta: PaneMeta;
  notification: PaneNotification | null;
  status: PaneStatus;
  exited: boolean;
  /** Id of the Claude Code conversation running here, reported by the hooks.
   *  Saved with the layout so a restore can bring the conversation back. */
  claude_session: string | null;
  /** A structured Claude chat covers this pane's terminal (see chat/). */
  chat?: boolean;
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

/// One tab = one named screen with its own split tree. Exactly one tab per
/// workspace is on screen; the rest stay mounted and hidden.
export interface TabInfo {
  id: TabId;
  name: string;
  layout: LayoutNode;
  active_pane: PaneId | null;
}

export interface WorkspaceInfo {
  id: WorkspaceId;
  name: string;
  tabs: TabInfo[];
  active_tab: TabId | null;
}

export interface NotificationEntry {
  pane: PaneId;
  tab_name: string;
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

// -- previous session ----------------------------------------------------------

/// What the last run left on disk. amux mirrors the *arrangement* (workspace
/// names, tab names, split shape, each pane's directory) to
/// `~/.config/amux/session.json` every second, so an unexpected exit costs the
/// shells but not the layout.
export interface SessionSummary {
  workspaces: number;
  tabs: number;
  panes: number;
  /** Unix epoch milliseconds of the last autosave. */
  saved_at_ms: number;
}

/** The previous session's size, or null when there is nothing to restore. */
export const savedSession = () => invoke<SessionSummary | null>("saved_session");

/** How a restore should answer Claude Code's "resume from a summary, or the
 *  full session?" menu on every pane it brings back. `ask` leaves the menu
 *  alone for the person to answer, which is what amux did before this existed. */
export type ResumeMode = "ask" | "full" | "summary";

/** Choices made once on the restore card and applied to every Claude pane. */
export interface ResumePrefs {
  /** Level for `claude --effort`, or null to leave it to the user's
   *  `~/.claude/settings.json`. */
  effort: string | null;
  mode: ResumeMode;
}

/// Rebuild the previous session's workspaces, tabs and splits in one call.
/// The panes come back as fresh shells in the directories they were in —
/// running processes and scrollback do not survive. Returns how many
/// workspaces were restored.
export const restoreSession = (prefs: ResumePrefs, cols = 80, rows = 24) =>
  invoke<number>("restore_session", { cols, rows, prefs });

// -- past conversations ---------------------------------------------------------

/// A Claude conversation that matched "지난 대화 검색" (or, for an empty query,
/// one of the most recent). Read from the transcripts Claude Code keeps under
/// `~/.claude/projects`; only what was typed and answered is indexed.
export interface HistoryHit {
  session: string;
  title: string | null;
  cwd: string | null;
  branch: string | null;
  /** ISO-8601 UTC of the conversation's last message. */
  last_ts: string | null;
  /** How many messages contain every search term. */
  matches: number;
  /** Index into the conversation's messages of the first match. */
  first_match: number | null;
  /** The first match with a little context, on one line. */
  snippet: string | null;
}

export interface HistoryMessage {
  role: "user" | "assistant";
  ts: string | null;
  text: string;
}

export interface HistoryConversation {
  session: string;
  title: string | null;
  cwd: string | null;
  branch: string | null;
  messages: HistoryMessage[];
}

/// Case-insensitive; every whitespace-separated term must appear in one
/// message. An empty query lists the most recent conversations.
export const searchHistory = (query: string, limit = 50) =>
  invoke<HistoryHit[]>("search_history", { query, limit });

export const historyConversation = (session: string) =>
  invoke<HistoryConversation | null>("history_conversation", { session });

// -- workspaces ---------------------------------------------------------------

/** `tabName` names the workspace's first tab; blank falls back to `탭 1`. */
export const createWorkspace = (name?: string, tabName?: string, cols = 80, rows = 24) =>
  invoke<WorkspaceId>("create_workspace", {
    name: name ?? null,
    tabName: tabName ?? null,
    cols,
    rows,
  });

export const closeWorkspace = (workspace: WorkspaceId) =>
  invoke<void>("close_workspace", { workspace });

export const focusWorkspace = (workspace: WorkspaceId) =>
  invoke<void>("focus_workspace", { workspace });

export const renameWorkspace = (workspace: WorkspaceId, name: string) =>
  invoke<void>("rename_workspace", { workspace, name });

export const moveWorkspace = (workspace: WorkspaceId, index: number) =>
  invoke<void>("move_workspace", { workspace, index });

/** `path` addresses a divider inside ONE tab's tree — the tab is not optional. */
export const setRatio = (
  workspace: WorkspaceId,
  tab: TabId,
  path: boolean[],
  ratio: number,
) => invoke<void>("set_ratio", { workspace, tab, path, ratio });

// -- tabs -----------------------------------------------------------------------

export const newTab = (workspace: WorkspaceId, name?: string, cols = 80, rows = 24) =>
  invoke<TabId>("new_tab", { workspace, name: name ?? null, cols, rows });

export const closeTab = (tab: TabId) => invoke<void>("close_tab", { tab });

export const focusTab = (tab: TabId) => invoke<void>("focus_tab", { tab });

export const renameTab = (tab: TabId, name: string) =>
  invoke<void>("rename_tab", { tab, name });

export const moveTab = (tab: TabId, index: number) =>
  invoke<void>("move_tab", { tab, index });

// -- panes ----------------------------------------------------------------------

export const splitPane = (pane: PaneId, axis: SplitAxis, cols = 80, rows = 24) =>
  invoke<PaneId>("split_pane", { pane, axis, cols, rows });

export const focusPane = (pane: PaneId) => invoke<void>("focus_pane", { pane });

export const renamePane = (pane: PaneId, name: string) =>
  invoke<void>("rename_pane", { pane, name });

/** Pin (`done = true`) or release the "finished, under review" status. */
export const setPaneDone = (pane: PaneId, done: boolean) =>
  invoke<void>("set_pane_done", { pane, done });

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

// --- Structured Claude chat ----------------------------------------------------
// The app runs `claude -p` with JSON lines on stdin/stdout per chat pane and
// relays every line as a `chat:event`; see src-tauri/src/chat.rs.

export interface ChatEventPayload {
  pane: PaneId;
  /** Which spawn of the pane's chat this belongs to (restarts get a new one). */
  gen: number;
  /** `line` = one JSON line from Claude, `stderr` = a log line, `exit` = gone. */
  kind: "line" | "stderr" | "exit";
  data: string;
}

export interface ChatStartOptions {
  model?: string | null;
  permissionMode?: string | null;
  /** Conversation id to continue (`claude --resume`). */
  resume?: string | null;
}

export const chatStart = (pane: PaneId, opts: ChatStartOptions = {}) =>
  invoke<number>("chat_start", {
    pane,
    model: opts.model ?? null,
    permissionMode: opts.permissionMode ?? null,
    resume: opts.resume ?? null,
  });

export const chatWrite = (pane: PaneId, gen: number, line: string) =>
  invoke<void>("chat_write", { pane, gen, line });

export const chatStop = (pane: PaneId) => invoke<void>("chat_stop", { pane });
/** Close the chat and give its conversation back to the terminal: `claude
 *  --resume <session>` is typed at the shell once the chat's Claude is gone.
 *  True when it typed (false: the shell was busy, or no transcript). */
export const chatHandback = (pane: PaneId, session: string) =>
  invoke<boolean>("chat_handback", { pane, session });

/** Generation of the chat process running in `pane`, if any. */
export const chatRunning = (pane: PaneId) => invoke<number | null>("chat_running", { pane });

// ---- a chat's working folder (src-tauri/src/workdir.rs), all read-only ----

export interface WorkRoot {
  /** The git repository's top folder, or the chat's folder outside git. */
  root: string;
  git: boolean;
  branch: string | null;
}

export interface WorkEntry {
  name: string;
  dir: boolean;
}

export type WorkChangeStatus = "modified" | "added" | "deleted" | "renamed" | "copied" | "new" | "conflict";

export interface WorkChange {
  /** Relative to the root, `/`-separated. */
  path: string;
  status: WorkChangeStatus;
  from: string | null;
  added: number | null;
  removed: number | null;
}

export interface WorkFile {
  kind: "text" | "binary" | "missing";
  text: string;
  truncated: boolean;
  size: number;
}

export const workRoot = (cwd: string) => invoke<WorkRoot>("work_root", { cwd });
/** One folder level; `rel` "" is the root. */
export const workList = (root: string, rel: string, git: boolean) =>
  invoke<WorkEntry[]>("work_list", { root, rel, git });
export const workStatus = (root: string) => invoke<WorkChange[]>("work_status", { root });
export const workRead = (root: string, rel: string) => invoke<WorkFile>("work_read", { root, rel });
/** The file as of the last commit; `missing` when it is not in it. */
export const workHead = (root: string, rel: string) => invoke<WorkFile>("work_head", { root, rel });

// -- files pop-out window (src-tauri/src/popout.rs) --------------------------

/** The main window's look, for the pop-out window to match. */
export interface Look {
  theme: string;
  fontSize: number;
}
export interface PopoutState {
  /** Panes whose files panel is in the pop-out window, in the order they came. */
  panes: PaneId[];
  look: Look | null;
}
/** Move a pane's files panel into the pop-out window (`out`) or back. */
export const filesPop = (pane: PaneId, out: boolean, look: Look | null) =>
  invoke<PopoutState>("files_pop", { pane, out, look });
export const filesState = () => invoke<PopoutState>("files_state");
export const filesLook = (look: Look) => invoke<void>("files_look", { look });

// -- subagents of the Claude in a pane (src-tauri/src/subagents.rs) ----------

/** A live Claude Code process found under a terminal pane's shell. */
export interface ClaudeLink {
  session: string;
  pid: number;
  /** It is working (not waiting for the person). */
  busy: boolean;
  /** The registry's status as written ("idle" while it waits for input). */
  status: string;
  /** When the process started (ms since the epoch). */
  started_ms: number;
}
/** One subagent's transcript file and what its meta file says. */
export interface AgentFile {
  id: string;
  agent_type: string | null;
  description: string | null;
  tool_use_id: string | null;
  finished: boolean;
  /** The person stopped it (its meta has `stoppedByUser`, no `finished`). */
  stopped: boolean;
  has_meta: boolean;
  size: number;
  created_ms: number;
  modified_ms: number;
}
/** What a subagent card shows of one transcript line. */
export type SubEntry =
  | { k: "prompt"; text: string; ts: string }
  | { k: "text"; text: string; ts: string }
  | { k: "tool"; id: string; name: string; input: Record<string, unknown> | null; ts: string }
  | { k: "result"; id: string; err: boolean; text: string; ts: string };
export interface SubChunk {
  items: SubEntry[];
  next: number;
  size: number;
  /** The file was rewritten: what was read before is void. */
  reset: boolean;
}
/** Pane id -> the Claude Code running in that terminal pane. */
export const claudeLinks = () => invoke<Record<PaneId, ClaudeLink> | null>("claude_links");
export const subagentsList = (session: string) => invoke<AgentFile[]>("subagents_list", { session });
export const subagentRead = (session: string, agent: string, from: number) =>
  invoke<SubChunk>("subagent_read", { session, agent, from });
/** The person's prompts added to the main transcript since `from` (null: just find its end). */
export const sessionPrompts = (session: string, from: number | null) =>
  invoke<{ count: number; next: number }>("session_prompts", { session, from });

// -- one conversation between a pane's terminal and its chat (src-tauri/src/handoff.rs)

/** The Claude Code running under this pane's shell right now. */
export const termClaudeLink = (pane: PaneId) => invoke<ClaudeLink | null>("term_claude_link", { pane });
/** End it (only while it waits for input and still runs `session`); resolves once it is gone. */
export const termClaudeEnd = (pane: PaneId, session: string) => invoke<void>("term_claude_end", { pane, session });
/** The conversation so far, as the chat draws it above its own turns. */
export interface SessionHistory {
  entries: SubEntry[];
  /** Earlier entries left out. */
  omitted: number;
  /** Compacted: what came before is only a summary to Claude now. */
  compacted: boolean;
}
export const sessionHistory = (session: string) => invoke<SessionHistory>("session_history", { session });
