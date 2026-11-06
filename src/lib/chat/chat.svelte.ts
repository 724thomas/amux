// Structured Claude chats, one per pane, and every action a chat view can take.
//
// Kept at module scope and keyed by pane id, for the same reason as
// `paneCommands` in state.svelte.ts: splitting or drag-moving a pane remounts
// its PaneView, and a conversation kept inside the component would be wiped
// by every split.
//
// The process side lives in src-tauri/src/chat.rs. This module builds every
// line sent to Claude (user messages, answers to permission cards, control
// requests) and feeds every line received into reducer.ts.
import { listen } from "@tauri-apps/api/event";
import {
  chatRunning,
  chatStart,
  chatStop,
  chatWrite,
  type ChatEventPayload,
  type PaneId,
} from "../ipc";
import {
  applyExit,
  applyLine,
  applyStderr,
  createModel,
  noteCardAnswered,
  noteControlFailed,
  noteClosed,
  noteControlSent,
  noteSendFailed,
  noteUserSent,
  pushNotice,
  type ChatModel,
} from "./reducer";

export interface ChatSession {
  /** The chat view is on screen, covering the pane's terminal. */
  open: boolean;
  /** Generation of the running process; null while none runs (or one starts). */
  gen: number | null;
  model: ChatModel;
  draft: string;
  /** The files panel: `auto` opens by itself when Claude starts editing a
   *  file (if there is room), `closed` means the user closed it, so it stays
   *  closed until they open it again. */
  panel: "auto" | "open" | "closed";
}

export const chats = $state<{ byPane: Record<PaneId, ChatSession> }>({ byPane: {} });

export function chatOf(pane: PaneId | null | undefined): ChatSession | undefined {
  return pane ? chats.byPane[pane] : undefined;
}

export function chatOpen(pane: PaneId | null | undefined): boolean {
  return chatOf(pane)?.open === true;
}

// Lines that arrive while `chat_start` is still in flight, before we know the
// new process's generation. Plain (not reactive): nobody renders them.
const early = new Map<PaneId, ChatEventPayload[]>();
const starting = new Map<PaneId, Promise<void>>();

let seq = 0;
const requestId = (kind: string) => `amux-${kind}-${Date.now().toString(36)}-${++seq}`;

function handle(s: ChatSession, ev: ChatEventPayload) {
  if (ev.kind === "line") {
    let obj: unknown;
    try {
      obj = JSON.parse(ev.data);
    } catch {
      return;
    }
    applyLine(s.model, obj);
  } else if (ev.kind === "stderr") {
    applyStderr(s.model, ev.data);
  } else if (ev.kind === "exit") {
    applyExit(s.model, ev.data);
    s.gen = null;
  }
}

let listening: Promise<unknown> | null = null;

/** Start listening for chat output. Idempotent; called once at app start. */
export function initChats() {
  listening ??= listen<ChatEventPayload>("chat:event", (e) => {
    const ev = e.payload;
    const s = chats.byPane[ev.pane];
    if (!s) return;
    if (s.gen === null) {
      const buf = early.get(ev.pane);
      if (buf && buf.length < 1000) buf.push(ev);
      return;
    }
    if (ev.gen === s.gen) handle(s, ev);
  });
  return listening;
}

/** The engine flag says this pane shows a chat, but this webview has no
 *  record of it: the page was reloaded. Re-attach to the running process
 *  (Claude still holds the conversation; only the drawn history is gone), or
 *  clear a flag whose process already ended. */
const adopting = new Set<PaneId>();
export async function adoptChat(pane: PaneId) {
  if (chats.byPane[pane] || adopting.has(pane)) return;
  adopting.add(pane);
  try {
    const gen = await chatRunning(pane);
    if (chats.byPane[pane]) return;
    if (gen == null) {
      await chatStop(pane);
      return;
    }
    const model = createModel();
    model.status = "idle";
    chats.byPane[pane] = { open: true, gen, model, draft: "", panel: "auto" };
    pushNotice(
      chats.byPane[pane].model,
      "info",
      "화면을 다시 불러와서 이전 대화 내용은 여기에 보이지 않습니다. Claude 는 대화를 그대로 기억하고 있으니 이어서 쓰면 됩니다.",
    );
  } catch {
    // The pane may be gone already; nothing to adopt.
  } finally {
    adopting.delete(pane);
  }
}

/** `--permission-mode` spells "default" as "manual". */
function flagMode(mode: string | null): string | null {
  if (!mode) return null;
  return mode === "default" ? "manual" : mode;
}

function startProcess(pane: PaneId, resume: string | null): Promise<void> {
  const pending = starting.get(pane);
  if (pending) return pending;
  const s = chats.byPane[pane];
  if (!s) return Promise.resolve();
  const run = (async () => {
    s.gen = null;
    s.model.status = "starting";
    s.model.exitCode = null;
    early.set(pane, []);
    try {
      const gen = await chatStart(pane, {
        resume,
        model: s.model.model,
        permissionMode: flagMode(s.model.permissionMode),
      });
      s.gen = gen;
      const buf = early.get(pane) ?? [];
      early.delete(pane);
      if (!s.open) {
        // Closed again while it was starting: don't leave it running unseen.
        s.gen = null;
        noteClosed(s.model);
        await chatStop(pane);
        return;
      }
      for (const ev of buf) if (ev.gen === gen && s.gen === gen) handle(s, ev);
      // The handshake fills the model picker (no model call, nothing billed).
      if (s.gen === gen) await control(pane, { subtype: "initialize" });
    } catch (e) {
      early.delete(pane);
      s.model.status = "exited";
      pushNotice(s.model, "error", `채팅을 시작하지 못했습니다: ${e}`);
    }
  })();
  starting.set(pane, run);
  void run.finally(() => starting.delete(pane));
  return run;
}

/** Show the chat in `pane`, starting Claude if it isn't running. A pane that
 *  chatted before picks its conversation back up. */
export async function openChat(pane: PaneId) {
  let s = chats.byPane[pane];
  if (!s) {
    chats.byPane[pane] = { open: true, gen: null, model: createModel(), draft: "", panel: "auto" };
    s = chats.byPane[pane];
  }
  s.open = true;
  if (s.gen !== null) return;
  await startProcess(pane, s.model.sessionId);
}

/** End the chat process and hand the pane back to its terminal. The drawn
 *  conversation stays, so reopening continues where it left off. */
export async function closeChat(pane: PaneId) {
  const s = chats.byPane[pane];
  if (s) {
    s.open = false;
    // Forget the process now; its late `exit` line then finds no match.
    if (s.gen !== null) {
      s.gen = null;
      noteClosed(s.model);
    }
  }
  try {
    await chatStop(pane);
  } catch {
    // Pane already gone.
  }
}

export function toggleChat(pane: PaneId | null | undefined) {
  if (!pane) return;
  if (chatOpen(pane)) void closeChat(pane);
  else void openChat(pane);
}

/** Start over with an empty conversation in the same pane. */
export async function newConversation(pane: PaneId) {
  const s = chats.byPane[pane];
  if (!s) return openChat(pane);
  const keep = { model: s.model.model, permissionMode: s.model.permissionMode, models: s.model.models };
  try {
    await chatStop(pane);
  } catch {
    // ignore
  }
  const fresh = createModel();
  fresh.model = keep.model;
  fresh.permissionMode = keep.permissionMode;
  fresh.models = keep.models;
  s.model = fresh;
  s.gen = null;
  s.open = true;
  await startProcess(pane, null);
}

/** Restart a chat whose process ended, continuing the same conversation. */
export async function restartChat(pane: PaneId) {
  const s = chats.byPane[pane];
  if (!s || s.gen !== null) return;
  await startProcess(pane, s.model.sessionId);
}

async function write(pane: PaneId, obj: unknown): Promise<void> {
  const s = chats.byPane[pane];
  if (!s || s.gen === null) throw new Error("채팅이 실행 중이 아닙니다");
  await chatWrite(pane, s.gen, JSON.stringify(obj));
}

async function control(pane: PaneId, request: { subtype: string } & Record<string, unknown>) {
  const s = chats.byPane[pane];
  if (!s) return false;
  const id = requestId(request.subtype);
  noteControlSent(s.model, id, request.subtype);
  try {
    await write(pane, { type: "control_request", request_id: id, request });
    return true;
  } catch (e) {
    noteControlFailed(s.model, id, String(e));
    return false;
  }
}

async function respond(pane: PaneId, id: string, response: Record<string, unknown>) {
  const s = chats.byPane[pane];
  try {
    await write(pane, {
      type: "control_response",
      response: { subtype: "success", request_id: id, response },
    });
  } catch (e) {
    if (s) pushNotice(s.model, "error", `응답을 보내지 못했습니다: ${e}`);
  }
}

/** Send one user message. Starts (or restarts) Claude first when needed;
 *  while a turn runs, the message queues and runs right after it. */
export async function sendMessage(pane: PaneId, text: string): Promise<boolean> {
  const s = chats.byPane[pane];
  const body = text.trim();
  if (!s || !body) return false;
  if (s.gen === null) await startProcess(pane, s.model.sessionId);
  if (s.gen === null) return false;
  noteUserSent(s.model, body);
  try {
    await write(pane, { type: "user", message: { role: "user", content: body } });
    return true;
  } catch (e) {
    noteSendFailed(s.model, String(e));
    return false;
  }
}

export type PermissionChoice = "allow" | "allowRemember" | "deny";

export async function answerPermission(pane: PaneId, itemId: number, choice: PermissionChoice) {
  const s = chats.byPane[pane];
  const item = s?.model.items[itemId];
  if (!s || !item || item.kind !== "permission" || item.state !== "pending") return;
  const input = JSON.parse(JSON.stringify(item.input));
  const suggestions = JSON.parse(JSON.stringify(item.suggestions));
  const response: Record<string, unknown> =
    choice === "deny"
      ? { behavior: "deny", message: "The user denied this tool use in the amux chat view." }
      : choice === "allowRemember" && suggestions.length > 0
        ? { behavior: "allow", updatedInput: input, updatedPermissions: suggestions }
        : { behavior: "allow", updatedInput: input };
  noteCardAnswered(s.model, itemId, choice === "deny" ? "denied" : "allowed");
  await respond(pane, item.requestId, response);
}

/** `answers` maps each question's text to the chosen label(s); null skips. */
export async function answerQuestion(pane: PaneId, itemId: number, answers: Record<string, string> | null) {
  const s = chats.byPane[pane];
  const item = s?.model.items[itemId];
  if (!s || !item || item.kind !== "question" || item.state !== "pending") return;
  const input = JSON.parse(JSON.stringify(item.input));
  const response: Record<string, unknown> = answers
    ? { behavior: "allow", updatedInput: { ...input, answers } }
    : { behavior: "deny", message: "The user skipped these questions." };
  noteCardAnswered(s.model, itemId, answers ? "answered" : "skipped", answers ?? undefined);
  await respond(pane, item.requestId, response);
}

export function interrupt(pane: PaneId) {
  const s = chats.byPane[pane];
  if (!s || s.gen === null || s.model.interrupting) return;
  void control(pane, { subtype: "interrupt" });
}

export function setModel(pane: PaneId, model: string) {
  void control(pane, { subtype: "set_model", model });
}

export function setPermissionMode(pane: PaneId, mode: string) {
  void control(pane, { subtype: "set_permission_mode", mode });
}

/** Drop the chats of panes that no longer exist (the processes are already
 *  stopped by the app when their pane closes). */
export function pruneChats(alive: Set<PaneId>) {
  for (const id of Object.keys(chats.byPane)) if (!alive.has(id)) delete chats.byPane[id];
}

// Keyboard routing: a chat view registers how to focus its composer, so an
// active-pane change by any means lands the keyboard in the chat, not in the
// terminal hidden under it.
const chatFocus = new Map<PaneId, () => void>();

export function registerChatFocus(pane: PaneId, focus: () => void): () => void {
  chatFocus.set(pane, focus);
  return () => {
    if (chatFocus.get(pane) === focus) chatFocus.delete(pane);
  };
}

/** Focus the pane's chat composer. False when the pane shows no chat. */
export function focusChat(pane: PaneId | null | undefined): boolean {
  if (!pane || !chatOpen(pane)) return false;
  const focus = chatFocus.get(pane);
  if (!focus) return false;
  focus();
  return true;
}
