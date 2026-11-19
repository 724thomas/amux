// The conversation model of one structured Claude chat, and how each JSON
// line from `claude -p --output-format stream-json` changes it.
//
// Plain TypeScript on purpose (no runes): chat.svelte.ts wraps a ChatModel in
// `$state`, so the mutations below are reactive there, and the same functions
// replay recorded streams in `bun test` without a DOM.
//
// Shape of the stream (verified against Claude Code 2.1.318):
// - `system/init` once per turn: session id, model, permission mode, cwd.
// - `stream_event` (with --include-partial-messages): Anthropic streaming
//   events. `content_block_start/delta/stop` carry `api_message_id`.
// - `assistant`: the finished content of ONE block (the CLI emits one
//   `assistant` per block, so a message with text + tool_use arrives twice).
// - `user`: tool results (`tool_result` blocks), or "[Request interrupted…]".
// - `control_request` / `can_use_tool`: a permission request, or an
//   AskUserQuestion, waiting for our `control_response`.
// - `control_response`: the answer to a request WE sent (initialize, …).
// - `result`: the turn ended (`success`, or `error_during_execution` on an
//   interrupt).
// Lines with a non-null `parent_tool_use_id` belong to a subagent; they never
// enter the main thread, they only tick the parent tool row.

import { isHunkList, type Hunk } from "./diff";
import { completeField } from "./partialJson";

export type ChatStatus = "starting" | "idle" | "working" | "waiting" | "exited";

/** Tools that change a file; their rows draw as live edit cards. */
export const EDIT_TOOLS = new Set(["Edit", "Write", "NotebookEdit", "MultiEdit"]);
const FILE_TOOLS = new Set([...EDIT_TOOLS, "Read"]);
/** Tools that only look: their results leave the folder as it was. */
const LOOK_TOOLS = new Set(["Read", "Grep", "Glob", "LS", "WebFetch", "WebSearch", "TodoWrite"]);

export interface ModelOption {
  value: string;
  displayName: string;
  description?: string;
  resolvedModel?: string;
  /** Effort levels this model takes (`supportedEffortLevels`); empty when it
   *  takes none, absent when Claude did not say. */
  effortLevels?: string[];
}

/** A slash command Claude offers (skills included), from `initialize`'s
 *  `commands`: typed as `/name args` it runs like in the terminal. */
export interface SlashCommand {
  name: string;
  description: string;
  argumentHint: string;
}

export interface QuestionOption {
  label: string;
  description?: string;
}

export interface Question {
  question: string;
  header?: string;
  options: QuestionOption[];
  multiSelect?: boolean;
}

export type PermissionState = "pending" | "allowed" | "denied" | "cancelled";

export type ChatItem =
  | { id: number; kind: "user"; text: string }
  | {
      id: number;
      kind: "assistant" | "thinking";
      text: string;
      streaming: boolean;
      final: boolean;
      msgId: string | null;
    }
  | {
      id: number;
      kind: "tool";
      toolUseId: string;
      name: string;
      input: Record<string, unknown>;
      /** Partial JSON of the input while it streams in. */
      inputJson: string;
      /** The input has fully arrived. */
      inputDone: boolean;
      /** The file this call reads or writes, known as soon as its path has
       *  fully arrived in the streaming input. */
      path: string | null;
      /** An applied edit's diff (`tool_use_result.structuredPatch`). */
      patch: Hunk[] | null;
      /** `create` or `update` for a Write, else null. */
      resultKind: string | null;
      result: string | null;
      isError: boolean;
      done: boolean;
      /** `fileSeq` when this call finished: a file view loaded at or after it
       *  already shows what the call did. */
      doneSeq: number;
      /** A permission card for this call is open. */
      awaiting: boolean;
      /** Subagent activity under this call (Agent/Task tools). */
      subSteps: number;
      subLast: string | null;
      /** A subagent's Write: the file as it was just before, read when the
       *  call arrived (its pop-out cell may open only after the write). */
      before?: string | null;
      /** A subagent's call: when it arrived (ms since the epoch), so that a
       *  cell opening a moment later still types it. */
      arrivedMs?: number;
    }
  | {
      id: number;
      kind: "permission";
      requestId: string;
      toolName: string;
      displayName: string;
      input: Record<string, unknown>;
      description: string | null;
      suggestions: Record<string, unknown>[];
      toolUseId: string | null;
      state: PermissionState;
    }
  | {
      id: number;
      kind: "question";
      requestId: string;
      toolUseId: string | null;
      input: Record<string, unknown>;
      questions: Question[];
      state: "pending" | "answered" | "skipped" | "cancelled";
      answers: Record<string, string> | null;
    }
  | {
      id: number;
      kind: "turn";
      ok: boolean;
      interrupted: boolean;
      subtype: string;
      durationMs: number | null;
    }
  | { id: number; kind: "notice"; tone: "info" | "error"; text: string };

export interface ChatModel {
  items: ChatItem[];
  status: ChatStatus;
  sessionId: string | null;
  model: string | null;
  permissionMode: string | null;
  models: ModelOption[];
  /** Slash commands and skills Claude takes, for the composer's `/` menu. */
  commands: SlashCommand[];
  /** The effort Claude applies now (`get_settings` → `applied.effort`). */
  effort: string | null;
  /** How much context the main thread's latest reply read (its input plus
   *  cache tokens), counted as Claude Code's status line counts `[ctx:N%]`;
   *  null before any reply. */
  ctxTokens: number | null;
  /** The model that reply came from (`message.model`), for its window. */
  ctxModel: string | null;
  /** That model's context window, from a turn's `result.modelUsage`. */
  ctxWindow: number | null;
  cwd: string | null;
  /** User messages sent whose `result` has not arrived yet. */
  pendingTurns: number;
  /** The CLI is between "requesting" and the first streamed token. */
  requesting: boolean;
  exitCode: string | null;
  stderrTail: string[];
  /** `${api_message_id}:${block index}` → item id of a streaming block. */
  streamKeys: Record<string, number>;
  /** tool_use_id → item id of its tool row. */
  toolIds: Record<string, number>;
  /** request_id → subtype, for the control requests we sent. */
  sentControl: Record<string, string>;
  /** We asked for an interrupt and its `result` has not landed yet. */
  interrupting: boolean;
  /** The file Claude is editing now, or edited last. */
  focus: { toolUseId: string; path: string; tool: string } | null;
  /** The file Claude read last. */
  lastRead: string | null;
  /** Absolute path → ids of the tool rows that edited it, oldest first. */
  edits: Record<string, number[]>;
  /** tool_use ids of file-changing calls, in the order their file became
   *  known (while their input was still streaming): the editor plays them in
   *  this order. */
  editOrder: string[];
  /** Bumped whenever something on disk may have changed (any finished tool
   *  call, a subagent's included, and every turn end): file views refresh. */
  fileSeq: number;
  /** Bumped on every message the user sends. */
  userSeq: number;
  /** The subagents this conversation sent off, by their Agent call's
   *  tool_use id, each with the file changes it made: a subagent that
   *  changed a file gets a files panel of its own (a pop-out cell). */
  subs: Record<string, SubFeed>;
  /** Bumped on every change to `subs` (a call, a result, its lifecycle). */
  subsSeq: number;
  /** Absolute path → how many applied subagent edits it had: the files panel
   *  of the conversation itself leaves those changes to the subagent's. */
  subTouched: Record<string, number>;
  /** Subagents' calls of LOOK_TOOLS still under way (by tool_use id): their
   *  results need no new look at the disk. */
  subLooks: Record<string, true>;
  /** Absolute path → when this conversation (its subagents too) last sent or
   *  got back an edit of it (ms since the epoch; the call comes a moment
   *  before the file changes, the result just after): the files panels of
   *  the other conversations take that change quietly instead of playing it. */
  editedAt: Record<string, number>;
}

/** One subagent: what it is, whether it is done, and its file-changing calls
 *  (only those), which the main window sends to its pop-out cell
 *  (subPanels.svelte.ts). Done, it stays until the person's next message. */
export interface SubFeed {
  id: string;
  description: string;
  /** Sent to the background: its Agent call answers at once, and it is done
   *  only at its `task_notification`. */
  backgrounded: boolean;
  finished: boolean;
  toolIds: Record<string, number>;
  items: ChatItem[];
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Json = any;

export function createModel(): ChatModel {
  return {
    items: [],
    status: "starting",
    sessionId: null,
    model: null,
    permissionMode: null,
    models: [],
    commands: [],
    effort: null,
    ctxTokens: null,
    ctxModel: null,
    ctxWindow: null,
    cwd: null,
    pendingTurns: 0,
    requesting: false,
    exitCode: null,
    stderrTail: [],
    streamKeys: {},
    toolIds: {},
    sentControl: {},
    interrupting: false,
    focus: null,
    lastRead: null,
    edits: {},
    editOrder: [],
    fileSeq: 0,
    userSeq: 0,
    subs: {},
    subsSeq: 0,
    subTouched: {},
    subLooks: {},
    editedAt: {},
  };
}

// `Omit` over a union keeps only the shared keys; this one omits per member.
type NewItem<T> = T extends unknown ? Omit<T, "id"> : never;

// Items are only ever appended, so an item's id is its index.
function push<T extends ChatItem>(m: ChatModel, item: NewItem<T>): T {
  const full = { ...item, id: m.items.length } as unknown as T;
  m.items.push(full);
  return m.items[full.id] as T;
}

function notice(m: ChatModel, tone: "info" | "error", text: string) {
  push(m, { kind: "notice", tone, text });
}

function toolRow(m: ChatModel, toolUseId: string, name: string) {
  const at = m.toolIds[toolUseId];
  if (at !== undefined) {
    const item = m.items[at];
    if (item.kind === "tool") {
      if (name && !item.name) item.name = name;
      return item;
    }
  }
  const item = push<Extract<ChatItem, { kind: "tool" }>>(m, {
    kind: "tool",
    toolUseId,
    name,
    input: {},
    inputJson: "",
    inputDone: false,
    path: null,
    patch: null,
    resultKind: null,
    result: null,
    isError: false,
    done: false,
    doneSeq: 0,
    awaiting: false,
    subSteps: 0,
    subLast: null,
  });
  m.toolIds[toolUseId] = item.id;
  return item;
}

type ToolItem = Extract<ChatItem, { kind: "tool" }>;

function inputPath(input: Json): string | null {
  const p = input?.file_path ?? input?.notebook_path;
  return typeof p === "string" && p ? p : null;
}

/** A file tool's target became known. */
function noteFile(m: ChatModel, row: ToolItem, path: string) {
  row.path = path;
  if (EDIT_TOOLS.has(row.name)) {
    m.focus = { toolUseId: row.toolUseId, path, tool: row.name };
    if (!m.editOrder.includes(row.toolUseId)) m.editOrder.push(row.toolUseId);
  }
  else if (row.name === "Read") m.lastRead = path;
}

/** Text of a tool_result's `content` (a string or a list of blocks). */
export function resultText(content: Json): string {
  if (typeof content === "string") return content;
  if (Array.isArray(content)) {
    return content
      .map((b: Json) => (b?.type === "text" ? String(b.text ?? "") : b?.type === "image" ? "[이미지]" : ""))
      .filter(Boolean)
      .join("\n");
  }
  return content == null ? "" : JSON.stringify(content);
}

/** The first streaming-or-unfinished block item of this message and kind. */
function openBlock(m: ChatModel, kind: "assistant" | "thinking", msgId: string) {
  for (const item of m.items) {
    if (item.kind === kind && item.msgId === msgId && !item.final) return item;
  }
  return null;
}

function finishStreaming(m: ChatModel) {
  for (const id of Object.values(m.streamKeys)) {
    const item = m.items[id];
    if (item && (item.kind === "assistant" || item.kind === "thinking")) item.streaming = false;
  }
  m.streamKeys = {};
}

function cancelOpenCards(m: ChatModel) {
  for (const item of m.items) {
    if ((item.kind === "permission" || item.kind === "question") && item.state === "pending") {
      item.state = "cancelled";
      setAwaiting(m, item.toolUseId, false);
    }
    if (item.kind === "tool") item.awaiting = false;
  }
}

function hasOpenCard(m: ChatModel): boolean {
  return m.items.some(
    (i) => (i.kind === "permission" || i.kind === "question") && i.state === "pending",
  );
}

/** Calls that never got a result (the turn was cut short) are over too;
 *  without this their rows would spin forever. */
function settleTools(m: ChatModel, why: string) {
  for (const item of m.items) {
    if (item.kind !== "tool" || item.done) continue;
    item.done = true;
    item.doneSeq = m.fileSeq;
    item.isError = true;
    item.awaiting = false;
    item.result = item.result ?? why;
  }
}

/** For a window that only watches a chat (the files pop-out window): answers
 *  to permission and question cards go to Claude's input, which such a window
 *  never sees. A card whose tool call has run, or whose turn is over, was
 *  answered there, so it must stop holding the status at "waiting". */
export function settleWatched(m: ChatModel, turnOver: boolean) {
  let changed = false;
  for (const item of m.items) {
    if ((item.kind !== "permission" && item.kind !== "question") || item.state !== "pending") continue;
    const at = item.toolUseId ? m.toolIds[item.toolUseId] : undefined;
    const tool = at === undefined ? undefined : m.items[at];
    if (turnOver || (tool?.kind === "tool" && tool.done)) {
      item.state = "cancelled";
      changed = true;
    }
  }
  if (changed) settleStatus(m);
}

function settleStatus(m: ChatModel) {
  if (m.status === "exited") return;
  if (hasOpenCard(m)) m.status = "waiting";
  else if (m.pendingTurns > 0) m.status = "working";
  else m.status = "idle";
}

function subagentTick(m: ChatModel, parent: string, obj: Json) {
  const at = m.toolIds[parent];
  if (at === undefined) return;
  const row = m.items[at];
  if (row.kind !== "tool") return;
  if (obj.type === "assistant") {
    for (const block of obj.message?.content ?? []) {
      if (block?.type === "tool_use") {
        row.subSteps += 1;
        row.subLast = String(block.name ?? "");
      } else if (block?.type === "text" && block.text) {
        row.subLast = String(block.text).split("\n")[0].slice(0, 80);
      }
    }
  }
}

// -- subagents' own files panels ---------------------------------------------

/** The subagent of Agent call `id`, made on first sight. */
function subOf(m: ChatModel, id: string): SubFeed {
  if (!m.subs[id]) {
    const at = m.toolIds[id];
    const row = at === undefined ? undefined : m.items[at];
    const input: Record<string, unknown> = row?.kind === "tool" ? row.input : {};
    m.subs[id] = {
      id,
      description: String(input.description ?? ""),
      backgrounded: false,
      finished: false,
      toolIds: {},
      items: [],
    };
  }
  return m.subs[id];
}

/** The subagent call row with this tool_use id, if a subagent made it. */
function subRowOf(m: ChatModel, toolUseId: string): { s: SubFeed; row: ToolItem } | null {
  if (!toolUseId) return null;
  for (const s of Object.values(m.subs)) {
    const at = s.toolIds[toolUseId];
    const row = at === undefined ? undefined : s.items[at];
    if (row?.kind === "tool") return { s, row };
  }
  return null;
}

/** A permission card for a call opened or closed: a subagent's call too. */
function setAwaiting(m: ChatModel, toolUseId: string | null, on: boolean) {
  if (!toolUseId) return;
  const at = m.toolIds[toolUseId];
  const row = at === undefined ? null : m.items[at];
  if (row && row.kind === "tool") {
    row.awaiting = on;
    return;
  }
  const hit = subRowOf(m, toolUseId);
  if (hit && hit.row.awaiting !== on) {
    hit.row.awaiting = on;
    m.subsSeq++;
  }
}

/** A subagent's line: its file-changing calls arrive whole (no streaming),
 *  then their results. */
function applySubLine(m: ChatModel, parent: string, obj: Json) {
  const content = obj.message?.content;
  if (!Array.isArray(content)) return;
  if (obj.type === "assistant") {
    for (const b of content) {
      if (b?.type !== "tool_use" || !EDIT_TOOLS.has(String(b.name))) continue;
      const id = String(b.id ?? "");
      const s = subOf(m, parent);
      if (!id || s.toolIds[id] !== undefined) continue;
      const input = (b.input ?? {}) as Record<string, unknown>;
      const path = inputPath(input);
      s.items.push({
        id: s.items.length,
        kind: "tool",
        toolUseId: id,
        name: String(b.name),
        input,
        inputJson: JSON.stringify(input),
        inputDone: true,
        path,
        patch: null,
        resultKind: null,
        result: null,
        isError: false,
        done: false,
        doneSeq: 0,
        awaiting: false,
        subSteps: 0,
        subLast: null,
        arrivedMs: Date.now(),
      });
      s.toolIds[id] = s.items.length - 1;
      // Before the file changes (other conversations' panels leave it alone).
      if (path) m.editedAt[path] = Date.now();
      m.subsSeq++;
    }
  } else if (obj.type === "user") {
    for (const b of content) {
      if (b?.type !== "tool_result") continue;
      const hit = subRowOf(m, String(b.tool_use_id ?? ""));
      if (!hit || hit.row.done) continue;
      const { row } = hit;
      row.isError = b.is_error === true;
      row.done = true;
      row.awaiting = false;
      if (!row.isError && row.path) {
        m.subTouched[row.path] = (m.subTouched[row.path] ?? 0) + 1;
        m.editedAt[row.path] = Date.now();
      }
      m.subsSeq++;
    }
  }
}

/** Claude went away: no subagent of it is working any more. */
function settleSubs(m: ChatModel) {
  m.subLooks = {};
  let changed = false;
  for (const s of Object.values(m.subs)) {
    for (const row of s.items) {
      if (row.kind === "tool" && (!row.done || row.awaiting)) {
        // Still waiting for the person's yes: it never ran.
        if (row.awaiting) row.isError = true;
        row.done = true;
        row.awaiting = false;
        changed = true;
      }
    }
    if (!s.finished) {
      s.finished = true;
      changed = true;
    }
  }
  if (changed) m.subsSeq++;
}

function applyStreamEvent(m: ChatModel, obj: Json) {
  const ev = obj.event ?? {};
  const msgId: string = obj.api_message_id ?? ev.message?.id ?? "?";
  const key = `${msgId}:${ev.index}`;
  switch (ev.type) {
    case "message_start":
      m.requesting = false;
      return;
    case "content_block_start": {
      const block = ev.content_block ?? {};
      if (block.type === "text" || block.type === "thinking") {
        const item = push<Extract<ChatItem, { kind: "assistant" | "thinking" }>>(m, {
          kind: block.type === "text" ? "assistant" : "thinking",
          text: String(block.text ?? block.thinking ?? ""),
          streaming: true,
          final: false,
          msgId,
        });
        m.streamKeys[key] = item.id;
      } else if (block.type === "tool_use" && block.id) {
        const row = toolRow(m, String(block.id), String(block.name ?? ""));
        m.streamKeys[key] = row.id;
      }
      return;
    }
    case "content_block_delta": {
      const at = m.streamKeys[key];
      if (at === undefined) return;
      const item = m.items[at];
      const d = ev.delta ?? {};
      if (d.type === "text_delta" && item.kind === "assistant") item.text += d.text ?? "";
      else if (d.type === "thinking_delta" && item.kind === "thinking") item.text += d.thinking ?? "";
      else if (d.type === "input_json_delta" && item.kind === "tool") {
        item.inputJson += d.partial_json ?? "";
        // The path comes first in these inputs; look for it only while the
        // input is short, so a long Write never pays a re-parse per piece.
        if (!item.path && FILE_TOOLS.has(item.name) && item.inputJson.length < 4096) {
          const p = completeField(item.inputJson, "file_path") ?? completeField(item.inputJson, "notebook_path");
          if (p) noteFile(m, item, p);
        }
      }
      return;
    }
    case "content_block_stop": {
      const at = m.streamKeys[key];
      if (at === undefined) return;
      const item = m.items[at];
      if (item.kind === "assistant" || item.kind === "thinking") item.streaming = false;
      else if (item.kind === "tool") {
        // Claude Code sends the message with the whole input just before this;
        // should it come later, the input is taken from the stream meanwhile
        // (a finished call with no input would look like an empty edit).
        if (Object.keys(item.input).length === 0 && item.inputJson) {
          try {
            const v = JSON.parse(item.inputJson);
            if (v && typeof v === "object" && !Array.isArray(v)) item.input = v as Record<string, unknown>;
          } catch {
            // Not whole after all: the message brings it.
          }
        }
        item.inputDone = true;
      }
      delete m.streamKeys[key];
      return;
    }
  }
}

/** The context a reply read, from its API usage: input plus cache creation
 *  plus cache read, output left out, as the status line's `[ctx:N%]` sums it.
 *  Claude Code's own stand-in replies (`<synthetic>`: an error, an
 *  interrupt) carry no real usage and are skipped. */
export function noteContext(m: ChatModel, msg: Json) {
  const u = msg?.usage;
  if (!u || typeof u !== "object" || msg.model === "<synthetic>") return;
  const read = tokens(u.input_tokens) + tokens(u.cache_creation_input_tokens) + tokens(u.cache_read_input_tokens);
  if (read <= 0) return;
  m.ctxTokens = read;
  if (typeof msg.model === "string" && msg.model) m.ctxModel = msg.model;
}

const tokens = (v: unknown) => (typeof v === "number" && Number.isFinite(v) && v > 0 ? v : 0);

/** The context window of `model` in a `result`'s `modelUsage`, which is keyed
 *  by model id and lists every model the session used (its subagents' too).
 *  With no entry for `model`, a lone entry is taken as the main one. */
export function contextWindowOf(modelUsage: Json, model: string | null): number | null {
  if (!modelUsage || typeof modelUsage !== "object") return null;
  const sized = Object.entries(modelUsage as Record<string, Json>).filter(
    ([, v]) => typeof v?.contextWindow === "number" && v.contextWindow > 0,
  );
  const hit = (model ? sized.find(([k]) => k === model) : undefined) ?? (sized.length === 1 ? sized[0] : undefined);
  return hit ? hit[1].contextWindow : null;
}

function applyAssistant(m: ChatModel, obj: Json) {
  const msg = obj.message ?? {};
  const msgId: string = msg.id ?? "?";
  m.requesting = false;
  noteContext(m, msg);
  for (const block of msg.content ?? []) {
    if (block?.type === "text") {
      const text = String(block.text ?? "");
      const open = openBlock(m, "assistant", msgId);
      if (open && open.kind === "assistant") {
        open.text = text;
        open.streaming = false;
        open.final = true;
      } else if (text.trim()) {
        push(m, { kind: "assistant", text, streaming: false, final: true, msgId });
      }
    } else if (block?.type === "thinking") {
      const text = String(block.thinking ?? "");
      const open = openBlock(m, "thinking", msgId);
      if (open && open.kind === "thinking") {
        open.text = text || open.text;
        open.streaming = false;
        open.final = true;
      } else if (text.trim()) {
        push(m, { kind: "thinking", text, streaming: false, final: true, msgId });
      }
    } else if (block?.type === "tool_use" && block.id) {
      const row = toolRow(m, String(block.id), String(block.name ?? ""));
      row.name = String(block.name ?? row.name);
      row.input = (block.input ?? {}) as Record<string, unknown>;
      row.inputJson = "";
      row.inputDone = true;
      const path = inputPath(row.input);
      if (path && FILE_TOOLS.has(row.name)) {
        noteFile(m, row, path);
        if (EDIT_TOOLS.has(row.name)) {
          const ids = (m.edits[path] ??= []);
          if (!ids.includes(row.id)) ids.push(row.id);
          // Before the file changes (the call runs after this message).
          m.editedAt[path] = Date.now();
        }
      }
    }
  }
}

function applyUser(m: ChatModel, obj: Json) {
  const content = obj.message?.content;
  if (typeof content === "string") {
    if (content.includes("[Request interrupted")) notice(m, "info", "중지했습니다.");
    return;
  }
  const results = (content ?? []).filter((b: Json) => b?.type === "tool_result");
  if (results.length > 0) m.fileSeq++;
  // `tool_use_result` (the tool's structured output) sits on the message, not
  // on a block; trust it only when the message carries exactly one result.
  const extra = results.length === 1 && obj.tool_use_result && typeof obj.tool_use_result === "object"
    ? obj.tool_use_result
    : null;
  for (const block of content ?? []) {
    if (block?.type === "tool_result") {
      const at = m.toolIds[String(block.tool_use_id ?? "")];
      if (at === undefined) continue;
      const row = m.items[at];
      if (row.kind !== "tool") continue;
      row.result = resultText(block.content);
      row.isError = block.is_error === true;
      row.done = true;
      row.doneSeq = m.fileSeq;
      row.awaiting = false;
      // An Agent call that ran in the foreground answers when its subagent
      // is done (a background one answers at once, and ends at its notice).
      const sub = m.subs[row.toolUseId];
      if (sub && !sub.backgrounded && !sub.finished) {
        sub.finished = true;
        m.subsSeq++;
      }
      if (extra && !row.isError) {
        if (isHunkList(extra.structuredPatch)) row.patch = extra.structuredPatch;
        if (extra.type === "create" || extra.type === "update") row.resultKind = extra.type;
        if (!row.path && typeof extra.filePath === "string" && FILE_TOOLS.has(row.name)) {
          noteFile(m, row, extra.filePath);
        }
      }
      if (!row.isError && row.path && EDIT_TOOLS.has(row.name)) m.editedAt[row.path] = Date.now();
    } else if (block?.type === "text" && String(block.text ?? "").includes("[Request interrupted")) {
      notice(m, "info", "중지했습니다.");
    }
  }
}

function applyControlRequest(m: ChatModel, obj: Json) {
  const req = obj.request ?? {};
  if (req.subtype !== "can_use_tool") return;
  const toolUseId: string | null = req.tool_use_id ?? null;
  const input = (req.input ?? {}) as Record<string, unknown>;
  setAwaiting(m, toolUseId, true);
  if (req.tool_name === "AskUserQuestion") {
    push(m, {
      kind: "question",
      requestId: String(obj.request_id),
      toolUseId,
      input,
      questions: (Array.isArray(input.questions) ? input.questions : []) as Question[],
      state: "pending",
      answers: null,
    });
  } else {
    push(m, {
      kind: "permission",
      requestId: String(obj.request_id),
      toolName: String(req.tool_name ?? ""),
      displayName: String(req.display_name ?? req.tool_name ?? ""),
      input,
      description: req.description ?? null,
      suggestions: (Array.isArray(req.permission_suggestions) ? req.permission_suggestions : []) as Record<
        string,
        unknown
      >[],
      toolUseId,
      state: "pending",
    });
  }
  m.status = "waiting";
}

function applyControlResponse(m: ChatModel, obj: Json) {
  const res = obj.response ?? {};
  const id = String(res.request_id ?? "");
  const sent = m.sentControl[id];
  delete m.sentControl[id];
  if (res.subtype === "error") {
    const what = sent?.startsWith("effort:") ? `effort ${sent.slice(7)} 적용` : sent;
    if (what) notice(m, "error", `${what} 요청 실패: ${res.error ?? "알 수 없는 오류"}`);
    return;
  }
  const body = res.response ?? {};
  // A live effort change (`apply_flag_settings`) answers with no body.
  if (sent?.startsWith("effort:")) {
    m.effort = sent.slice(7);
    return;
  }
  if (sent === "initialize") {
    if (Array.isArray(body.models)) {
      m.models = body.models.map((x: Json) => ({
        value: String(x.value),
        displayName: String(x.displayName ?? x.value),
        description: x.description,
        resolvedModel: x.resolvedModel,
        effortLevels:
          x.supportsEffort === true && Array.isArray(x.supportedEffortLevels)
            ? x.supportedEffortLevels.map(String)
            : x.supportsEffort === false
              ? []
              : undefined,
      }));
    }
    if (Array.isArray(body.commands)) {
      m.commands = body.commands
        .filter((c: Json) => c && typeof c.name === "string" && c.name && !c.name.startsWith("__"))
        .map((c: Json) => ({
          name: String(c.name),
          description: String(c.description ?? ""),
          argumentHint: String(c.argumentHint ?? ""),
        }));
    }
    if (body.current_permission_mode) m.permissionMode = String(body.current_permission_mode);
    if (m.status === "starting") m.status = "idle";
  } else if (sent === "get_settings") {
    // `applied` is what the running session uses (a start flag or a live
    // change included); `effective` is only the merged settings files.
    const applied = body.applied?.effort ?? body.effective?.effortLevel;
    if (typeof applied === "string") m.effort = applied;
  } else if (sent === "set_permission_mode" && body.mode) {
    m.permissionMode = String(body.mode);
  } else if (sent === "set_model" && body.model) {
    m.model = String(body.model);
  }
}

function applyResult(m: ChatModel, obj: Json) {
  finishStreaming(m);
  m.fileSeq++;
  const subtype = String(obj.subtype ?? "");
  const ok = subtype === "success" && obj.is_error !== true;
  const interrupted = !ok && m.interrupting;
  push(m, {
    kind: "turn",
    ok,
    interrupted,
    subtype,
    durationMs: typeof obj.duration_ms === "number" ? obj.duration_ms : null,
  });
  if (!ok && !interrupted) {
    const why = typeof obj.result === "string" && obj.result.trim() ? obj.result.trim() : subtype;
    notice(m, "error", `이번 차례가 오류로 끝났습니다: ${why}`);
  }
  if (interrupted) cancelOpenCards(m);
  settleTools(m, interrupted ? "중지되어 실행되지 않았습니다." : "결과 없이 차례가 끝났습니다.");
  m.interrupting = false;
  m.requesting = false;
  m.pendingTurns = Math.max(0, m.pendingTurns - 1);
  if (obj.session_id) m.sessionId = String(obj.session_id);
  const window = contextWindowOf(obj.modelUsage, m.ctxModel ?? m.model);
  if (window) m.ctxWindow = window;
  settleStatus(m);
}

/** Apply one parsed stdout line. */
export function applyLine(m: ChatModel, obj: Json) {
  if (!obj || typeof obj !== "object") return;
  const parent = obj.parent_tool_use_id;
  if (parent) {
    subagentTick(m, String(parent), obj);
    applySubLine(m, String(parent), obj);
    // A subagent's tool may have changed files too, unless it only looked (a
    // subagent that searches can return dozens of results a minute, and each
    // look at the disk costs two git runs and a read of every changed file).
    const blocks = Array.isArray(obj.message?.content) ? obj.message.content : [];
    if (obj.type === "assistant") {
      for (const b of blocks) if (b?.type === "tool_use" && b.id && LOOK_TOOLS.has(String(b.name))) m.subLooks[String(b.id)] = true;
    } else if (obj.type === "user") {
      let changed = false;
      for (const b of blocks) {
        if (b?.type !== "tool_result") continue;
        const id = String(b.tool_use_id ?? "");
        if (m.subLooks[id]) delete m.subLooks[id];
        else changed = true;
      }
      if (changed) m.fileSeq++;
    }
    return;
  }
  switch (obj.type) {
    case "system":
      if (obj.subtype === "init") {
        m.sessionId = obj.session_id ?? m.sessionId;
        m.model = obj.model ?? m.model;
        m.permissionMode = obj.permissionMode ?? m.permissionMode;
        m.cwd = obj.cwd ?? m.cwd;
        // `init` opens every turn. Trust it over our own count: a message
        // queued behind an interrupt still runs, and must not show as idle.
        m.pendingTurns = Math.max(m.pendingTurns, 1);
        if (m.status !== "waiting" && m.status !== "exited") m.status = "working";
      } else if (obj.subtype === "status") {
        if (obj.permissionMode) m.permissionMode = String(obj.permissionMode);
        m.requesting = obj.status === "requesting";
      } else if (obj.subtype === "api_retry") {
        notice(m, "info", `API 재시도 중 (${obj.attempt}/${obj.max_retries}${obj.error ? `, ${obj.error}` : ""})`);
      } else if (obj.subtype === "task_started" && obj.tool_use_id && (obj.subagent_type || obj.task_type === "local_agent")) {
        // A subagent set off (a background one's Agent call answers at once).
        const s = subOf(m, String(obj.tool_use_id));
        if (obj.description) s.description = String(obj.description);
        s.backgrounded = obj.is_backgrounded === true;
        m.subsSeq++;
      } else if (obj.subtype === "task_notification" && obj.tool_use_id) {
        // A subagent ended (completed, failed or stopped alike).
        const s = m.subs[String(obj.tool_use_id)];
        if (s && !s.finished) {
          s.finished = true;
          m.subsSeq++;
        }
      }
      return;
    case "stream_event":
      applyStreamEvent(m, obj);
      return;
    case "assistant":
      applyAssistant(m, obj);
      return;
    case "user":
      applyUser(m, obj);
      return;
    case "control_request":
      applyControlRequest(m, obj);
      return;
    case "control_cancel_request": {
      for (const item of m.items) {
        if (
          (item.kind === "permission" || item.kind === "question") &&
          item.requestId === String(obj.request_id) &&
          item.state === "pending"
        ) {
          item.state = "cancelled";
          setAwaiting(m, item.toolUseId, false);
        }
      }
      settleStatus(m);
      return;
    }
    case "control_response":
      applyControlResponse(m, obj);
      return;
    case "result":
      applyResult(m, obj);
      return;
    case "rate_limit_event": {
      const info = obj.rate_limit_info ?? {};
      if (info.status && info.status !== "allowed" && info.status !== "allowed_warning") {
        notice(m, "error", `사용 한도에 걸렸습니다 (${info.rateLimitType ?? info.status}).`);
      }
      return;
    }
  }
}

// -- what we did ourselves -------------------------------------------------------

export function pushNotice(m: ChatModel, tone: "info" | "error", text: string) {
  notice(m, tone, text);
}

/** A message we showed as sent never reached the process. */
export function noteSendFailed(m: ChatModel, why: string) {
  m.pendingTurns = Math.max(0, m.pendingTurns - 1);
  m.requesting = false;
  notice(m, "error", `보내지 못했습니다: ${why}`);
  settleStatus(m);
}

/** A control request we noted never reached the process. */
export function noteControlFailed(m: ChatModel, requestId: string, why: string) {
  const subtype = m.sentControl[requestId];
  delete m.sentControl[requestId];
  if (subtype === "interrupt") m.interrupting = false;
  notice(m, "error", `${subtype ?? "제어"} 요청을 보내지 못했습니다: ${why}`);
}

/** What the files panel reads: a chat's model, or the stand-in a terminal
 *  pane builds from its status (no tool calls, only "something may have
 *  changed" and "a new turn started"). */
export type FileFeed = Pick<ChatModel, "editOrder" | "toolIds" | "items" | "fileSeq" | "status" | "userSeq"> & {
  /** The conversation's own panel: changes its subagents made, left to theirs. */
  subTouched?: Record<string, number>;
  /** When the conversation's edits landed, by path (see ChatModel). */
  editedAt?: Record<string, number>;
};

/** When the edits of every conversation but `self` last landed, by path (the
 *  latest of them): a files panel leaves those changes to their own panels. */
export function editedElsewhere(models: Iterable<FileFeed>, self: FileFeed): Record<string, number> {
  const out: Record<string, number> = {};
  for (const m of models) {
    if (m === self || !m.editedAt) continue;
    for (const [p, t] of Object.entries(m.editedAt)) if (!(out[p] >= t)) out[p] = t;
  }
  return out;
}

export function noteUserSent(m: ChatModel, text: string) {
  push(m, { kind: "user", text });
  m.userSeq += 1;
  m.pendingTurns += 1;
  m.requesting = true;
  if (m.status !== "waiting") m.status = "working";
  // Subagents that are done go with this message (their cards close too).
  dropFinishedSubs(m);
}

/** A new request: the subagents that are done go (with their calls, a
 *  Write's whole content among them); the ones still working stay. The files
 *  pop-out window, which keeps a model of its own, calls it when it sees a
 *  turn start. */
export function dropFinishedSubs(m: ChatModel) {
  let gone = false;
  for (const [id, s] of Object.entries(m.subs)) {
    if (!s.finished) continue;
    delete m.subs[id];
    gone = true;
  }
  if (gone) m.subsSeq++;
}

export function noteControlSent(m: ChatModel, requestId: string, subtype: string) {
  m.sentControl[requestId] = subtype;
  if (subtype === "interrupt") m.interrupting = true;
}

export function noteCardAnswered(
  m: ChatModel,
  itemId: number,
  state: "allowed" | "denied" | "answered" | "skipped",
  answers?: Record<string, string>,
) {
  const item = m.items[itemId];
  if (!item) return;
  if (item.kind === "permission" && (state === "allowed" || state === "denied")) {
    item.state = state;
  } else if (item.kind === "question" && (state === "answered" || state === "skipped")) {
    item.state = state;
    item.answers = answers ?? null;
  } else {
    return;
  }
  setAwaiting(m, item.toolUseId, false);
  if (!hasOpenCard(m)) m.status = m.pendingTurns > 0 ? "working" : "idle";
}

/** The person closed the chat: its process is going away on purpose. */
export function noteClosed(m: ChatModel) {
  finishStreaming(m);
  // Before the cards go, which clears what was still waiting for a yes.
  settleSubs(m);
  cancelOpenCards(m);
  m.status = "exited";
  m.pendingTurns = 0;
  m.requesting = false;
  m.interrupting = false;
  m.exitCode = null;
  notice(m, "info", "채팅을 닫았습니다. 다시 열면 같은 대화를 이어 갑니다.");
}

export function applyStderr(m: ChatModel, line: string) {
  m.stderrTail.push(line);
  if (m.stderrTail.length > 20) m.stderrTail.splice(0, m.stderrTail.length - 20);
}

export function applyExit(m: ChatModel, code: string) {
  finishStreaming(m);
  // Before the cards go, which clears what was still waiting for a yes.
  settleSubs(m);
  cancelOpenCards(m);
  m.fileSeq++;
  settleTools(m, "Claude 가 끝나 실행되지 않았습니다.");
  m.exitCode = code;
  m.status = "exited";
  m.pendingTurns = 0;
  m.requesting = false;
  const tail = m.stderrTail.slice(-5).join("\n");
  if (code && code !== "0") {
    notice(m, "error", `Claude 가 종료됐습니다 (종료 코드 ${code}).${tail ? "\n" + tail : ""}`);
  } else {
    notice(m, "info", "Claude 세션이 끝났습니다.");
  }
}
