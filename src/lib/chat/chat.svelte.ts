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
  chatHandback,
  chatRunning,
  chatStart,
  chatStop,
  chatWrite,
  sessionHistory,
  termClaudeEnd,
  termClaudeLink,
  type ChatEventPayload,
  type ClaudeLink,
  type PaneId,
  type SubEntry,
} from "../ipc";
import { paneInfo } from "../state.svelte";
import { subs } from "./subagentColumn.svelte";
import { lastTermOf } from "./subagentLinks.svelte";
import { running } from "./subagents";
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
  /** The files panel: `auto` opens by itself when Claude starts changing a
   *  file, `closed` means the person closed it (it opens by itself again
   *  from their next message on, see `holdSeq`). */
  panel: "auto" | "open" | "closed";
  /** The request (`model.userSeq`) in which the person closed the panel or
   *  put it back from the pop-out window: until their next message the
   *  panel is left as they put it. -1: none. */
  holdSeq: number;
  /** When the running process was started (ms since the epoch), null when
   *  not known (adopted after a reload). The subagent column uses it to tell
   *  this run's subagents from ones an earlier run left unfinished. */
  startedMs: number | null;
  /** The chat took its conversation over from a terminal Claude it ended:
   *  closing the chat gives it back (`claude --resume` typed there). */
  tookOver: boolean;
  /** When the chat was last closed (ms), to tell whether a terminal Claude
   *  ran in the pane after it. */
  closedAt: number;
  /** The conversation so far, drawn above the chat's own turns, when the
   *  chat picked up a terminal Claude's session (null: none). */
  history: ChatHistory | null;
}

export interface ChatHistory {
  session: string;
  state: "loading" | "ready" | "error";
  entries: SubEntry[];
  /** Earlier entries left out. */
  omitted: number;
  /** Compacted: what came before is only a summary to Claude now. */
  compacted: boolean;
  error: string | null;
}

function newSession(model: ChatModel = createModel()): ChatSession {
  return {
    open: true,
    gen: null,
    model,
    draft: "",
    panel: "auto",
    holdSeq: -1,
    startedMs: null,
    tookOver: false,
    closedAt: 0,
    history: null,
  };
}

// A short line over a pane: how the hand-over between terminal and chat went
// (or why it did not happen). Goes away by itself.
export const paneNotes = $state<{ byPane: Record<PaneId, { text: string; tone: "info" | "error"; seq: number }> }>({
  byPane: {},
});
let noteSeq = 0;
export function notePane(pane: PaneId, text: string, tone: "info" | "error" = "info", ms = 6000) {
  const seq = ++noteSeq;
  paneNotes.byPane[pane] = { text, tone, seq };
  setTimeout(() => {
    if (paneNotes.byPane[pane]?.seq === seq) delete paneNotes.byPane[pane];
  }, ms);
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
    chats.byPane[pane] = { ...newSession(model), gen };
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
      const startedMs = Date.now();
      const gen = await chatStart(pane, {
        resume,
        model: s.model.model,
        permissionMode: flagMode(s.model.permissionMode),
      });
      s.gen = gen;
      s.startedMs = startedMs;
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

/** Panes whose terminal conversation is being handed to the chat. */
const handing = new Set<PaneId>();

/** Show the chat in `pane`, starting Claude if it isn't running. The pane's
 *  conversation goes on in it, the same session:
 *    a Claude Code running in the terminal under the chat is ended (only
 *    while it waits for input) and its session continues here;
 *    one that ran and ended after the chat was last closed is picked up;
 *    otherwise the chat's own conversation continues (as before). */
export async function openChat(pane: PaneId) {
  const before = chats.byPane[pane];
  if (before && before.gen !== null) {
    before.open = true;
    return;
  }
  if (handing.has(pane)) return;
  handing.add(pane);
  try {
    let link: ClaudeLink | null = null;
    try {
      link = await termClaudeLink(pane);
    } catch {
      link = null; // an older backend
    }
    if (link) {
      const status = paneInfo(pane)?.status;
      if (link.status !== "idle" || status === "processing" || status === "waiting") {
        notePane(
          pane,
          "터미널의 Claude 가 일하는 중이라 채팅으로 넘기지 않았습니다. 끝난 뒤 다시 눌러 주세요.",
          "error",
        );
        return;
      }
      // A subagent it sent off in the background may still be working while
      // the main conversation waits for input: ending Claude would end it too.
      const col = subs.byPane[pane];
      if (col?.session === link.session && col.agents.some(running)) {
        notePane(
          pane,
          "터미널의 Claude 가 맡긴 서브에이전트가 아직 일하는 중이라 채팅으로 넘기지 않았습니다. 끝난 뒤 다시 눌러 주세요.",
          "error",
        );
        return;
      }
      notePane(pane, "터미널의 Claude 대화를 채팅으로 넘겨받는 중…", "info", 15000);
      try {
        await termClaudeEnd(pane, link.session);
      } catch (e) {
        notePane(pane, `채팅으로 넘기지 못했습니다: ${e}`, "error");
        return;
      }
      takeOver(pane, link.session, true);
      delete paneNotes.byPane[pane];
    } else {
      const seen = lastTermOf(pane);
      if (seen && seen.at > (before?.closedAt ?? 0)) takeOver(pane, seen.session, false);
    }
    let s = chats.byPane[pane];
    if (!s) {
      chats.byPane[pane] = newSession();
      s = chats.byPane[pane];
    }
    s.open = true;
    if (s.gen !== null) return;
    await startProcess(pane, s.model.sessionId);
  } finally {
    handing.delete(pane);
  }
}

/** The chat continues `session`, a terminal Claude's: a fresh model on it
 *  (the model and mode picks kept), and the conversation so far loaded from
 *  its transcript to draw above. `ended`: we ended that Claude, so closing
 *  the chat gives the conversation back to the terminal. */
function takeOver(pane: PaneId, session: string, ended: boolean) {
  const old = chats.byPane[pane];
  const model = createModel();
  if (old) {
    model.model = old.model.model;
    model.permissionMode = old.model.permissionMode;
    model.models = old.model.models;
  }
  model.sessionId = session;
  chats.byPane[pane] = {
    ...newSession(model),
    draft: old?.draft ?? "",
    panel: old?.panel ?? "auto",
    closedAt: old?.closedAt ?? 0,
    tookOver: ended,
    history: { session, state: "loading", entries: [], omitted: 0, compacted: false, error: null },
  };
  const s = chats.byPane[pane];
  sessionHistory(session).then(
    (h) => {
      if (s.history?.session !== session) return;
      s.history = { ...s.history, state: "ready", entries: h.entries, omitted: h.omitted, compacted: h.compacted };
    },
    (e) => {
      if (s.history?.session !== session) return;
      s.history = { ...s.history, state: "error", error: String(e) };
    },
  );
}

/** End the chat process and hand the pane back to its terminal. The drawn
 *  conversation stays, so reopening continues where it left off. */
export async function closeChat(pane: PaneId) {
  const s = chats.byPane[pane];
  // Taken over from the terminal's Claude: give the conversation back there.
  const giveBack = s?.tookOver ? s.model.sessionId : null;
  if (s) {
    s.open = false;
    s.closedAt = Date.now();
    s.tookOver = false;
    // Forget the process now; its late `exit` line then finds no match.
    if (s.gen !== null) {
      s.gen = null;
      noteClosed(s.model);
    }
  }
  if (giveBack) {
    try {
      const typed = await chatHandback(pane, giveBack);
      if (!typed) {
        notePane(
          pane,
          `터미널이 다른 일을 하고 있어 Claude 를 다시 열지 않았습니다. 이어 하려면 claude --resume ${giveBack}`,
          "info",
          15000,
        );
      }
    } catch (e) {
      notePane(pane, `터미널에서 대화를 다시 열지 못했습니다: ${e}`, "error");
    }
    return;
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
  // Its requests count from 0 again.
  s.holdSeq = -1;
  // A new conversation: nothing taken over any more, nothing to give back.
  s.history = null;
  s.tookOver = false;
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
