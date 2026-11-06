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

export interface ModelOption {
  value: string;
  displayName: string;
  description?: string;
  resolvedModel?: string;
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

function applyAssistant(m: ChatModel, obj: Json) {
  const msg = obj.message ?? {};
  const msgId: string = msg.id ?? "?";
  m.requesting = false;
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
      if (extra && !row.isError) {
        if (isHunkList(extra.structuredPatch)) row.patch = extra.structuredPatch;
        if (extra.type === "create" || extra.type === "update") row.resultKind = extra.type;
        if (!row.path && typeof extra.filePath === "string" && FILE_TOOLS.has(row.name)) {
          noteFile(m, row, extra.filePath);
        }
      }
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
  if (toolUseId) {
    const at = m.toolIds[toolUseId];
    const row = at === undefined ? null : m.items[at];
    if (row && row.kind === "tool") row.awaiting = true;
  }
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
    if (sent) notice(m, "error", `${sent} 요청 실패: ${res.error ?? "알 수 없는 오류"}`);
    return;
  }
  const body = res.response ?? {};
  if (sent === "initialize") {
    if (Array.isArray(body.models)) {
      m.models = body.models.map((x: Json) => ({
        value: String(x.value),
        displayName: String(x.displayName ?? x.value),
        description: x.description,
        resolvedModel: x.resolvedModel,
      }));
    }
    if (body.current_permission_mode) m.permissionMode = String(body.current_permission_mode);
    if (m.status === "starting") m.status = "idle";
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
  settleStatus(m);
}

/** Apply one parsed stdout line. */
export function applyLine(m: ChatModel, obj: Json) {
  if (!obj || typeof obj !== "object") return;
  const parent = obj.parent_tool_use_id;
  if (parent) {
    subagentTick(m, String(parent), obj);
    // A subagent's tool may have changed files too.
    const blocks = obj.type === "user" ? obj.message?.content : null;
    if (Array.isArray(blocks) && blocks.some((b: Json) => b?.type === "tool_result")) m.fileSeq++;
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
export type FileFeed = Pick<ChatModel, "editOrder" | "toolIds" | "items" | "fileSeq" | "status" | "userSeq">;

export function noteUserSent(m: ChatModel, text: string) {
  push(m, { kind: "user", text });
  m.userSeq += 1;
  m.pendingTurns += 1;
  m.requesting = true;
  if (m.status !== "waiting") m.status = "working";
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
  if (item.toolUseId) {
    const at = m.toolIds[item.toolUseId];
    const row = at === undefined ? null : m.items[at];
    if (row && row.kind === "tool") row.awaiting = false;
  }
  if (!hasOpenCard(m)) m.status = m.pendingTurns > 0 ? "working" : "idle";
}

/** The person closed the chat: its process is going away on purpose. */
export function noteClosed(m: ChatModel) {
  finishStreaming(m);
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
