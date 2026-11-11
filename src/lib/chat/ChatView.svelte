<script lang="ts">
  // A structured Claude chat drawn over a pane's terminal: the conversation as
  // a page (markdown replies, tool calls as rows, permission requests and
  // questions as cards) instead of the Claude Code TUI. State and actions live
  // in chat.svelte.ts; this component only draws them and routes input.
  import { onMount, tick, untrack, type Snippet } from "svelte";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { writeText } from "@tauri-apps/plugin-clipboard-manager";
  import type { PaneId } from "../ipc";
  import { paneInfo, paneOnScreen } from "../state.svelte";
  import { settings } from "../settings.svelte";
  import {
    answerPermission,
    answerQuestion,
    chatOf,
    closeChat,
    editedBesides,
    interrupt,
    newConversation,
    registerChatFocus,
    restartChat,
    sendMessage,
    setModel,
    setPermissionMode,
    setEffort,
    type PermissionChoice,
  } from "./chat.svelte";
  import { EDIT_TOOLS, type ChatItem, type Question, type SlashCommand } from "./reducer";
  import EditCard from "./EditCard.svelte";
  import FilesPanel from "./FilesPanel.svelte";
  import Dropdown from "./Dropdown.svelte";
  import { dockBack, popOut, popout } from "./popout.svelte";
  import { renderMarkdown } from "./markdown";
  import { clip, inputJson, seconds, shortPath, toolSummary } from "./format";
  import { historyRows } from "./history";
  import { stepDiff } from "./subagents";

  let {
    pane,
    focused,
    tools,
  }: {
    pane: PaneId;
    focused: boolean;
    /** The pane's own actions (done, move, split, close), drawn in the header. */
    tools?: Snippet<[boolean]>;
  } = $props();

  const s = $derived(chatOf(pane));
  const m = $derived(s?.model);
  const status = $derived(m?.status ?? "starting");
  const busy = $derived(status === "working" || status === "waiting");
  const cwd = $derived(m?.cwd ?? paneInfo(pane)?.meta.cwd ?? null);
  const last = $derived(m && m.items.length ? m.items[m.items.length - 1] : null);
  const streamingText = $derived(
    last != null && (last.kind === "assistant" || last.kind === "thinking") && last.streaming,
  );

  // ---- the files panel (FilesPanel.svelte) ----
  let bodyWidth = $state(0);
  /** The panel's share of the width beside the conversation: files 3,
   *  conversation 2 (until the person drags its edge). */
  const PANEL_SHARE = 0.6;
  /** The conversation keeps at least this much beside the panel. */
  const MIN_CONVERSATION = 300;
  /** Room for the conversation and the panel side by side, at 3:2. */
  const wide = $derived(bodyWidth >= MIN_CONVERSATION / (1 - PANEL_SHARE));
  /** This chat's files panel is in the pop-out window (then not shown here). */
  const popped = $derived(popout.panes.includes(pane));
  const panelOpen = $derived(s?.panel === "open" && !popped);
  // On a narrow pane the panel takes the conversation's place, but never
  // while a permission or question card is waiting there for an answer.
  const panelVisible = $derived(panelOpen && (wide || status !== "waiting"));
  const covers = $derived(panelVisible && !wide);
  /** This chat's workspace and tab are the ones on screen. */
  const onScreen = $derived(paneOnScreen(pane));
  const editing = $derived.by(() => {
    const f = m?.focus;
    if (!f || !m) return false;
    const at = m.toolIds[f.toolUseId];
    const row = at === undefined ? undefined : m.items[at];
    return row?.kind === "tool" && !row.done;
  });
  const editedCount = $derived(m ? Object.keys(m.edits).length : 0);
  function setPanel(state: "open" | "closed") {
    if (!s) return;
    s.panel = state;
    if (state === "closed" && m) s.holdSeq = m.userSeq;
  }
  // A file change started (an Edit/Write call, or a change seen on disk):
  // show the files by themselves. With the pop-out window open they go there,
  // as a cell of their own (the window keeps every chat's calls, so the one
  // being written is typed there from its start); else beside the
  // conversation (over it, in a narrow pane). What the person did during this
  // request (closed the panel, put it back from the pop-out) stands until
  // their next message.
  function wantPanel() {
    if (!s || !m || popped) return;
    if (s.holdSeq === m.userSeq) return;
    s.panel = "open";
    if (popout.panes.length > 0) popOut(pane);
  }
  // Put back from the pop-out window (its ↩, or the window closed): shown
  // here, and kept here for the rest of this request.
  let wasPopped = untrack(() => popped);
  $effect(() => {
    const now = popped;
    untrack(() => {
      if (wasPopped && !now && s && m) {
        s.panel = "open";
        s.holdSeq = m.userSeq;
      }
      wasPopped = now;
    });
  });
  /** Height of the bar under the conversation: an overlaid panel stops above it. */
  let bottomHeight = $state(0);

  const STATUS_LABEL: Record<string, string> = {
    starting: "시작 중",
    idle: "대기",
    working: "작업 중",
    waiting: "확인 필요",
    exited: "종료됨",
  };

  const MODE_LABEL: Record<string, string> = {
    default: "매번 묻기",
    acceptEdits: "편집은 자동 허용",
    auto: "자동 판단",
    plan: "계획만 세우기",
    dontAsk: "미리 허용된 것만",
    bypassPermissions: "모두 허용 (위험)",
  };
  const MODE_CHOICES = ["default", "acceptEdits", "auto", "plan"];
  // Tools whose card body already shows what the request's description says
  // (the command and its purpose, or the file path).
  const SELF_DESCRIBED = new Set(["Bash", "Write", "Edit", "MultiEdit"]);

  const modeOptions = $derived.by(() => {
    const cur = m?.permissionMode;
    const list = [...MODE_CHOICES];
    if (cur && !list.includes(cur)) list.push(cur);
    return list.map((v) => ({ value: v, label: MODE_LABEL[v] ?? v }));
  });

  const modelOptions = $derived.by(() => {
    const out = (m?.models ?? []).map((o) => ({ value: o.value, label: o.displayName }));
    const cur = m?.model;
    if (cur && !(m?.models ?? []).some((o) => o.value === cur || o.resolvedModel === cur)) {
      out.unshift({ value: cur, label: cur });
    }
    return out;
  });
  const modelValue = $derived.by(() => {
    const cur = m?.model;
    if (!cur) return "";
    const hit = (m?.models ?? []).find((o) => o.value === cur || o.resolvedModel === cur);
    return hit ? hit.value : cur;
  });
  // Effort: the levels the current model takes (from Claude's model list), all
  // of them when Claude did not say, none when it says the model takes none.
  const EFFORT_ORDER = ["low", "medium", "high", "xhigh", "max"];
  const effortOptions = $derived.by(() => {
    const models = m?.models ?? [];
    const hit = models.find((o) => o.value === modelValue || (m?.model != null && o.resolvedModel === m.model));
    const levels = hit?.effortLevels ?? EFFORT_ORDER;
    const list = EFFORT_ORDER.filter((v) => levels.includes(v));
    const cur = m?.effort;
    if (cur && !list.includes(cur)) list.push(cur);
    return list.map((v) => ({ value: v, label: v === "max" ? "effort max (다시 시작)" : `effort ${v}` }));
  });
  const controlsLive = $derived(s?.gen != null && status !== "starting" && status !== "exited");

  /** Long changes in the taken-over history the person opened in full. */
  let historyOpen = $state<Record<number, boolean>>({});

  // ---- scrolling: follow the bottom unless the user scrolled up to read ----
  let scroller = $state<HTMLDivElement>();
  let list = $state<HTMLDivElement>();
  let stick = true;
  let lastTop = 0;
  function onScroll() {
    if (!scroller) return;
    const top = scroller.scrollTop;
    // Scroll events arrive a frame late, when more content may already have
    // grown the list, so distance-from-bottom alone would read our own
    // auto-scroll as the user leaving. Only an upward move is the user's.
    if (scroller.scrollHeight - top - scroller.clientHeight < 48) stick = true;
    else if (top < lastTop - 4) stick = false;
    lastTop = top;
    nearTop();
  }
  $effect(() => {
    const el = list;
    if (!el) return;
    const ro = new ResizeObserver(() => {
      // Not while the mouse is held in the conversation: a selection being
      // dragged would run away with the text scrolling under the pointer.
      if (stick && scroller && !pressing) scroller.scrollTop = scroller.scrollHeight;
    });
    ro.observe(el);
    return () => ro.disconnect();
  });

  // ---- what is drawn: the last few exchanges; older ones as you scroll up ----
  // Every drawn item makes each screen update of WebKit dearer (measured with
  // target/perf/wk_bench.py: a key cost 2ms at 252 items, 26ms at 2,004), and
  // the whole app shares that one thread, so a long chat used to slow typing
  // until amux froze. The conversation itself (`m.items`) stays whole: only
  // the drawing is limited, so files, edits and Claude's memory are untouched.
  /** Exchanges (a message of the person's and all that answered it) drawn by default. */
  const BASE_TURNS = 3;
  /** At most this many items by default, should one exchange be enormous
   *  (one prompt, hundreds of tool calls). Measured in WebKitGTK: 250 items
   *  cost what 21 do, 600 already a third more per streamed piece. */
  const CAP = 250;
  /** At most this many items added by one scroll to the top. */
  const STEP = 300;
  /** First chat item drawn once the person scrolled up for more; null: the default. */
  let itemFrom = $state<number | null>(null);
  /** First taken-over history row drawn once scrolled up for more; null: the default. */
  let histFrom = $state<number | null>(null);

  const histRows = $derived(s?.history ? historyRows(s.history.entries) : []);
  /** Where the last `BASE_TURNS` exchanges start, and how many of them the
   *  chat itself holds (the rest come from the taken-over history). */
  const defaultWindow = $derived.by(() => {
    const items = m?.items ?? [];
    let users = 0;
    let from = 0;
    for (let i = items.length - 1; i >= 0; i--) {
      if (items[i].kind === "user" && ++users === BASE_TURNS) {
        from = i;
        break;
      }
    }
    return { from: Math.max(from, items.length - CAP), users };
  });
  const itemFirst = $derived(Math.min(itemFrom ?? Infinity, defaultWindow.from));
  const shownItems = $derived(m ? (itemFirst > 0 ? m.items.slice(itemFirst) : m.items) : []);
  const defaultHistFrom = $derived.by(() => {
    const need = BASE_TURNS - defaultWindow.users;
    if (histRows.length === 0 || defaultWindow.from > 0 || need <= 0) return histRows.length;
    let users = 0;
    for (let i = histRows.length - 1; i >= 0; i--) {
      if (histRows[i].kind === "user" && ++users === need) return Math.max(i, histRows.length - CAP);
    }
    return Math.max(0, histRows.length - CAP);
  });
  const histFirst = $derived(Math.min(histFrom ?? Infinity, defaultHistFrom));
  /** The taken-over history is older than every chat item: drawn only once
   *  all of those are. */
  const historyDrawn = $derived(itemFirst === 0 && (histFirst < histRows.length || s?.history?.state !== "ready"));
  const moreAbove = $derived(itemFirst > 0 || (histRows.length > 0 && histFirst > 0));

  // A new conversation (or one taken over) starts from the default again.
  $effect(() => {
    void m;
    untrack(() => {
      itemFrom = null;
      histFrom = null;
      recall = null;
    });
  });

  /** Draw one more exchange above (at most `STEP` items), keeping what is on
   *  screen where it is: WebKit does not hold the view when content grows
   *  above it, so the added height goes onto `scrollTop`. */
  let loadingMore = false;
  async function loadMore() {
    if (!m || !scroller || loadingMore || !moreAbove) return;
    if (itemFirst > 0) {
      let i = itemFirst - 1;
      while (i > 0 && m.items[i].kind !== "user") i--;
      itemFrom = Math.max(i, itemFirst - STEP);
    } else {
      let i = Math.min(histFirst, histRows.length) - 1;
      while (i > 0 && histRows[i].kind !== "user") i--;
      histFrom = Math.max(i, histFirst - STEP);
    }
    loadingMore = true;
    const before = scroller.scrollHeight;
    const top = scroller.scrollTop;
    await tick();
    if (scroller) {
      scroller.scrollTop = top + (scroller.scrollHeight - before);
      lastTop = scroller.scrollTop;
    }
    loadingMore = false;
  }
  /** Near the top (or at it, with nothing to scroll): bring in the next exchange. */
  function nearTop() {
    if (scroller && moreAbove && scroller.scrollTop < 200) void loadMore();
  }
  // Too little drawn to scroll at all (a short default on a tall pane): fill
  // up until it scrolls, or nothing is left above. Not while hidden.
  $effect(() => {
    void shownItems.length;
    void histFirst;
    if (!scroller || !moreAbove) return;
    void tick().then(() => {
      if (scroller && scroller.clientHeight > 0 && scroller.scrollHeight <= scroller.clientHeight + 8) void loadMore();
    });
  });

  // ---- keyboard ----
  let input = $state<HTMLTextAreaElement>();
  onMount(() => registerChatFocus(pane, () => input?.focus()));
  // A press in the conversation may be the start of a text selection: the
  // pane becoming focused from that same press must not pull the keyboard
  // into the composer, which would drop the selection being dragged.
  let pressing = false;
  $effect(() => {
    if (focused && input) untrack(() => !pressing && !selectedText() && input?.focus());
  });

  // ---- copying the conversation ----
  /** The text selected inside this conversation, or "" (none, or elsewhere). */
  function selectedText(): string {
    const sel = window.getSelection();
    if (!sel || sel.isCollapsed || !scroller || !sel.anchorNode) return "";
    if (!scroller.contains(sel.anchorNode) || !sel.focusNode || !scroller.contains(sel.focusNode)) return "";
    return sel.toString();
  }
  // Ctrl+C (and the terminal habit Ctrl+Shift+C) copy what is selected in the
  // conversation, through the Rust clipboard like the terminal does
  // (navigator.clipboard is unreliable in WebKitGTK). A selection inside the
  // composer keeps its native copy.
  function onCopyKey(e: KeyboardEvent) {
    if (!e.ctrlKey || e.altKey || e.metaKey || e.code !== "KeyC") return;
    if (document.activeElement === input && input && input.selectionStart !== input.selectionEnd) return;
    const text = selectedText();
    if (!text) return;
    e.preventDefault();
    e.stopPropagation();
    void writeText(text);
  }

  // Fit the box to its text, as cheaply as typing allows. Every height change
  // re-lays out the whole conversation beside it (thousands of nodes in a long
  // chat), and collapsing to `auto` to measure is one more forced layout, so:
  // growing just reads the overflow, only a shorter text (or a new font size)
  // collapses to measure, and nothing is written when the height is the same.
  // Not while Korean is being composed either: the IME's preedit would be
  // disturbed (flicker, late letters); the end of composition fits it.
  let composing = false;
  let fitLen = 0;
  let fitFont = 0;
  function autosize() {
    if (!input || composing) return;
    const len = input.value.length;
    const font = settings.fontSize;
    let h: number;
    if (len >= fitLen && font === fitFont && input.style.height) {
      if (input.scrollHeight <= input.clientHeight) {
        fitLen = len;
        return;
      }
      h = input.scrollHeight;
    } else {
      const was = input.style.height;
      input.style.height = "auto";
      h = input.scrollHeight;
      input.style.height = was;
    }
    fitLen = len;
    fitFont = font;
    const px = Math.min(h, 220) + "px";
    if (input.style.height !== px) input.style.height = px;
  }
  $effect(() => {
    void s?.draft;
    void settings.fontSize;
    queueMicrotask(autosize);
  });

  async function submit() {
    if (!s) return;
    const text = s.draft;
    if (!text.trim()) return;
    s.draft = "";
    stick = true;
    // Back to the last few exchanges: reading far back does not keep it heavy.
    itemFrom = null;
    histFrom = null;
    recall = null;
    const ok = await sendMessage(pane, text);
    if (!ok && s && !s.draft) s.draft = text;
  }

  // ---- earlier prompts: ↑ / ↓ in the composer, like a shell or the Claude CLI ----
  /** A walk back through the person's earlier prompts, newest first. `at` is
   *  the one in the composer (-1: their own unsent text, kept in `saved`).
   *  Plain, nothing is drawn from it; typing or sending ends the walk. */
  let recall: { list: string[]; at: number; saved: string } | null = null;
  /** The person's prompts in this conversation, newest first, each once: the
   *  chat's own and the taken-over history's (drawn or not). Built only when
   *  a walk starts, so streaming never pays for it. */
  function earlierPrompts(): string[] {
    const all: string[] = [];
    for (const r of histRows) if (r.kind === "user") all.push(r.text);
    for (const it of m?.items ?? []) if (it.kind === "user") all.push(it.text);
    const seen = new Set<string>();
    const list: string[] = [];
    for (let i = all.length - 1; i >= 0; i--) {
      const t = all[i];
      if (!t.trim() || seen.has(t)) continue;
      seen.add(t);
      list.push(t);
    }
    return list;
  }
  function showRecalled(text: string, up: boolean) {
    if (!s) return;
    s.draft = text;
    // A recalled `/command` does not pop the `/` menu up (it would take ↑ / ↓).
    slashHiddenFor = text;
    // The caret goes to the end (to edit and send), except a several-line
    // prompt reached with ↑: at its start, so the next ↑ goes on back.
    const atStart = up && text.includes("\n");
    void tick().then(() => {
      if (!input) return;
      const p = atStart ? 0 : input.value.length;
      input.setSelectionRange(p, p);
    });
  }
  /** ↑ on the first line and ↓ on the last line walk the earlier prompts
   *  (true: handled); anywhere else the arrows move the caret as usual. */
  function onRecallKey(e: KeyboardEvent): boolean {
    if (!s || !input || e.shiftKey || e.altKey || e.ctrlKey || e.metaKey) return false;
    const a = input.selectionStart;
    const b = input.selectionEnd;
    if (a !== b) return false;
    const v = input.value;
    if (e.key === "ArrowUp") {
      if (v.slice(0, a).includes("\n")) return false;
      recall ??= { list: earlierPrompts(), at: -1, saved: s.draft };
      if (recall.at + 1 >= recall.list.length) {
        // Nothing older (or nothing at all): the arrow does what it always does.
        if (recall.at < 0) recall = null;
        return false;
      }
      recall.at++;
      showRecalled(recall.list[recall.at], true);
      return true;
    }
    if (e.key === "ArrowDown" && recall) {
      if (v.slice(b).includes("\n")) return false;
      recall.at--;
      if (recall.at >= 0) showRecalled(recall.list[recall.at], false);
      else {
        // Past the newest: back to what the person had typed.
        const back = recall.saved;
        recall = null;
        showRecalled(back, false);
      }
      return true;
    }
    return false;
  }

  // ---- the `/` menu: slash commands and skills, like the Claude CLI ----
  /** Esc hides the menu for the text it was open on; other text shows it again. */
  let slashHiddenFor = $state<string | null>(null);
  let slashIndex = $state(0);
  let slashList = $state<HTMLElement | null>(null);
  /** What follows the `/` while the composer holds only `/name-so-far`
   *  (no space yet); null: no menu. */
  const slashQuery = $derived.by(() => {
    const d = s?.draft ?? "";
    if (!d.startsWith("/") || d === slashHiddenFor) return null;
    const q = d.slice(1);
    return /\s/.test(q) ? null : q.toLowerCase();
  });
  /** Names starting with it first (an exact name on top), then names holding it. */
  const slashMatches = $derived.by(() => {
    const q = slashQuery;
    const all = m?.commands ?? [];
    if (q === null || all.length === 0) return [];
    const exact: SlashCommand[] = [];
    const starts: SlashCommand[] = [];
    const within: SlashCommand[] = [];
    for (const c of all) {
      const n = c.name.toLowerCase();
      if (n === q) exact.push(c);
      else if (n.startsWith(q)) starts.push(c);
      else if (n.includes(q)) within.push(c);
    }
    return [...exact, ...starts, ...within];
  });
  const slashOpen = $derived(slashMatches.length > 0);
  // A new search starts at the top.
  $effect(() => {
    void slashQuery;
    untrack(() => (slashIndex = 0));
  });
  // Keep the picked one in view.
  $effect(() => {
    const i = slashIndex;
    if (!slashOpen || !slashList) return;
    (slashList.children[i] as HTMLElement | undefined)?.scrollIntoView({ block: "nearest" });
  });
  function pickSlash(c: SlashCommand) {
    if (!s) return;
    s.draft = "/" + c.name + " ";
    void tick().then(() => {
      if (!input) return;
      input.focus();
      const p = input.value.length;
      input.setSelectionRange(p, p);
    });
  }
  /** Keys while the menu is open (true: handled). ↑ ↓ move, Tab or Enter
   *  fills the name in (Enter on a name typed out in full sends it), Esc hides. */
  function onSlashKey(e: KeyboardEvent): boolean {
    if (!slashOpen || e.altKey || e.ctrlKey || e.metaKey) return false;
    const n = slashMatches.length;
    if (e.key === "ArrowDown") slashIndex = (slashIndex + 1) % n;
    else if (e.key === "ArrowUp") slashIndex = (slashIndex - 1 + n) % n;
    else if ((e.key === "Tab" || e.key === "Enter") && !e.shiftKey) {
      const c = slashMatches[Math.min(slashIndex, n - 1)];
      if (e.key === "Enter" && c.name.toLowerCase() === slashQuery) return false;
      pickSlash(c);
    } else if (e.key === "Escape") slashHiddenFor = s?.draft ?? null;
    else return false;
    return true;
  }

  function onKey(e: KeyboardEvent) {
    // 한글 IME 조합 중의 Enter 는 글자 확정이지 전송이 아니다 (Terminal.svelte 의
    // 입력창과 같은 가드).
    if (e.isComposing || e.keyCode === 229) return;
    if (onSlashKey(e)) {
      e.preventDefault();
      return;
    }
    if (e.key === "Enter" && !e.shiftKey && !e.altKey && !e.ctrlKey && !e.metaKey) {
      e.preventDefault();
      void submit();
    } else if (e.key === "Escape" && busy) {
      e.preventDefault();
      interrupt(pane);
    } else if ((e.key === "ArrowUp" || e.key === "ArrowDown") && onRecallKey(e)) {
      e.preventDefault();
    }
  }

  // Links open in the system browser; navigating would take amux itself away.
  function onLinkClick(e: MouseEvent) {
    const a = (e.target as HTMLElement | null)?.closest?.("a");
    if (!a) return;
    e.preventDefault();
    const href = a.getAttribute("href") ?? "";
    if (/^(https?:|mailto:)/i.test(href)) void openUrl(href);
  }

  // ---- tool rows ----
  let expanded = $state<Record<number, boolean>>({});
  const toggle = (id: number) => (expanded[id] = !expanded[id]);

  // ---- cards ----
  function choose(id: number, choice: PermissionChoice) {
    void answerPermission(pane, id, choice);
    input?.focus();
  }

  type Suggestion = Record<string, unknown>;
  function suggestionLabel(list: Suggestion[]): string {
    const where: Record<string, string> = {
      session: "이번 대화 동안",
      localSettings: "이 폴더에서 앞으로",
      projectSettings: "이 프로젝트에서 앞으로",
      userSettings: "모든 곳에서 앞으로",
    };
    const parts = list.map((sg) => {
      const dest = where[String(sg.destination ?? "")] ?? "앞으로";
      if (sg.type === "setMode") return `${dest} ${MODE_LABEL[String(sg.mode)] ?? String(sg.mode)}`;
      if (sg.type === "addRules" || sg.type === "replaceRules") {
        const rules = (Array.isArray(sg.rules) ? sg.rules : [])
          .map((r: Record<string, unknown>) => (r.ruleContent ? `${r.toolName}(${r.ruleContent})` : String(r.toolName)))
          .join(", ");
        return `${dest} ${rules} 허용`;
      }
      if (sg.type === "addDirectories") {
        const dirs = (Array.isArray(sg.directories) ? sg.directories : []).map((d) => shortPath(String(d)));
        return `${dest} ${dirs.join(", ")} 폴더 허용`;
      }
      return `${dest} 비슷한 요청 허용`;
    });
    return parts.join(", ");
  }

  // Question cards: picked labels per (item, question), plus a free answer.
  let picks = $state<Record<string, string[]>>({});
  let others = $state<Record<string, string>>({});
  const qkey = (id: number, qi: number) => `${id}:${qi}`;
  function pick(id: number, qi: number, label: string, multi: boolean | undefined) {
    const key = qkey(id, qi);
    const cur = picks[key] ?? [];
    if (multi) picks[key] = cur.includes(label) ? cur.filter((l) => l !== label) : [...cur, label];
    else picks[key] = cur.includes(label) ? [] : [label];
  }
  function answerOf(id: number, qi: number): string {
    const key = qkey(id, qi);
    const own = (others[key] ?? "").trim();
    return [...(picks[key] ?? []), ...(own ? [own] : [])].join(", ");
  }
  function submitQuestions(id: number, questions: Question[]) {
    const answers: Record<string, string> = {};
    for (let qi = 0; qi < questions.length; qi++) answers[questions[qi].question] = answerOf(id, qi);
    void answerQuestion(pane, id, answers);
    input?.focus();
  }
  function allAnswered(id: number, questions: Question[]) {
    return questions.every((_, qi) => answerOf(id, qi) !== "");
  }

  // ---- mermaid diagrams, drawn once a reply is complete ----
  let mermaidLoad: Promise<typeof import("mermaid").default> | null = null;
  let mermaidSeq = 0;
  function darkBackground(): boolean {
    const bg = getComputedStyle(document.documentElement).getPropertyValue("--bg").trim();
    const hex = bg.match(/^#([0-9a-f]{6})$/i)?.[1];
    if (!hex) return true;
    const [r, g, b] = [0, 2, 4].map((i) => parseInt(hex.slice(i, i + 2), 16));
    return 0.299 * r + 0.587 * g + 0.114 * b < 128;
  }
  function drawFailed(el: HTMLElement, why: string) {
    const note = document.createElement("div");
    note.className = "mermaid-error";
    note.textContent = `그림을 그리지 못했습니다: ${why}`;
    el.appendChild(note);
  }

  // An ```svg block becomes an <img> of a data URL: an SVG shown as an image
  // runs no script and loads nothing from outside, whatever its source says.
  function drawSvg(el: HTMLElement) {
    el.dataset.drawn = "1";
    let src = (el.dataset.src ?? "").trim();
    if (src.length > 300_000) return drawFailed(el, "SVG 가 너무 큽니다");
    const doc = new DOMParser().parseFromString(src, "image/svg+xml");
    if (doc.querySelector("parsererror") || doc.documentElement.localName !== "svg") {
      return drawFailed(el, "SVG 문법 오류");
    }
    const open = src.match(/<svg\b[^>]*>/);
    if (open && !/\sxmlns\s*=/.test(open[0])) {
      src = src.replace(/<svg\b/, '<svg xmlns="http://www.w3.org/2000/svg"');
    }
    const img = new Image();
    img.alt = "Claude 가 그린 그림";
    img.onload = () => {
      el.replaceChildren(img);
      el.classList.add("drawn");
    };
    img.onerror = () => drawFailed(el, "이미지로 열리지 않습니다");
    img.src = "data:image/svg+xml;charset=utf-8," + encodeURIComponent(src);
  }

  async function drawDiagrams(root: HTMLElement) {
    for (const el of root.querySelectorAll<HTMLElement>(".svg-block[data-src]:not([data-drawn])")) drawSvg(el);
    const blocks = [...root.querySelectorAll<HTMLElement>(".mermaid-block[data-src]:not([data-drawn])")];
    if (!blocks.length) return;
    mermaidLoad ??= import("mermaid").then((mod) => mod.default);
    const mermaid = await mermaidLoad;
    mermaid.initialize({
      startOnLoad: false,
      securityLevel: "strict",
      theme: darkBackground() ? "dark" : "default",
      suppressErrorRendering: true,
    });
    for (const el of blocks) {
      if (!el.isConnected) continue;
      el.dataset.drawn = "1";
      try {
        const { svg } = await mermaid.render(`amux-mermaid-${++mermaidSeq}`, el.dataset.src ?? "");
        el.innerHTML = svg;
        el.classList.add("drawn");
      } catch (err) {
        drawFailed(el, String((err as Error)?.message ?? err).split("\n")[0]);
      }
    }
  }
  function diagrams(node: HTMLElement, done: boolean) {
    const run = (d: boolean) => {
      if (d) requestAnimationFrame(() => void drawDiagrams(node));
    };
    run(done);
    return { update: run };
  }

  function turnLabel(item: Extract<ChatItem, { kind: "turn" }>): string {
    if (item.ok) return item.durationMs != null ? `완료 (${seconds(item.durationMs)})` : "완료";
    if (item.interrupted) return "중지됨";
    return "오류로 끝남";
  }
</script>

<!-- Captured: ahead of the app shortcuts and of a terminal holding the keys. -->
<svelte:window onkeydowncapture={onCopyKey} />

{#snippet toolInput(name: string, input: Record<string, unknown>)}
  {#if name === "Bash"}
    <pre class="cmd">$ {String(input.command ?? "")}</pre>
    {#if input.description}<div class="muted small">{String(input.description)}</div>{/if}
  {:else if name === "Edit"}
    <div class="file">{shortPath(String(input.file_path ?? ""), cwd)}</div>
    <pre class="del">{clip(String(input.old_string ?? ""), 4000)}</pre>
    <pre class="add">{clip(String(input.new_string ?? ""), 4000)}</pre>
  {:else if name === "MultiEdit" && Array.isArray(input.edits)}
    <div class="file">{shortPath(String(input.file_path ?? ""), cwd)}</div>
    {#each input.edits.slice(0, 10) as edit, i (i)}
      <pre class="del">{clip(String(edit?.old_string ?? ""), 2000)}</pre>
      <pre class="add">{clip(String(edit?.new_string ?? ""), 2000)}</pre>
    {/each}
  {:else if name === "Write"}
    <div class="file">{shortPath(String(input.file_path ?? ""), cwd)}</div>
    <pre class="add">{clip(String(input.content ?? ""), 4000)}</pre>
  {:else}
    <pre>{clip(inputJson(input), 4000)}</pre>
  {/if}
{/snippet}

{#if s && m}
  <div class="chat" style="--chat-fs: {settings.fontSize}px">
    <header>
      <span class="status {status}" title="Claude 상태">
        <span class="dot"></span>{STATUS_LABEL[status] ?? status}
      </span>
      <span class="cwd" title={cwd ?? ""}>{cwd ? shortPath(cwd) : ""}</span>
      <Dropdown
        title="Effort: Claude 가 얼마나 깊이 생각할지 (높을수록 느리고 꼼꼼함, 다음 응답부터 적용). max 는 차례 사이에 Claude 를 같은 대화로 다시 시작합니다."
        value={m.effort ?? ""}
        options={effortOptions}
        placeholder="effort"
        disabled={!controlsLive || effortOptions.length === 0}
        onchange={(v) => void setEffort(pane, v)}
      />
      <Dropdown
        title="모델 (다음 응답부터 적용)"
        value={modelValue}
        options={modelOptions}
        placeholder="모델"
        disabled={!controlsLive || modelOptions.length === 0}
        onchange={(v) => setModel(pane, v)}
      />
      <Dropdown
        title="권한 모드: 도구를 쓰기 전에 언제 물어볼지"
        value={m.permissionMode ?? ""}
        options={modeOptions}
        placeholder="권한"
        disabled={!controlsLive}
        onchange={(v) => setPermissionMode(pane, v)}
      />
      <button
        class="ghost files-btn"
        class:on={panelOpen || popped}
        class:busy={editing}
        title={popped ? "파일 창이 팝업 창에 있습니다 (누르면 다시 여기로)" : "작업 폴더와 바뀐 파일을 옆에 보기"}
        onmousedown={(e) => e.preventDefault()}
        onclick={() => {
          if (popped) {
            setPanel("open");
            dockBack(pane);
          } else setPanel(panelOpen ? "closed" : "open");
        }}>{popped ? "⧉ 팝업" : "📂 파일"}{editedCount ? ` ${editedCount}` : ""}</button
      >
      <button
        class="ghost"
        title="지금 대화를 끝내고 새 대화 시작"
        onmousedown={(e) => e.preventDefault()}
        onclick={() => void newConversation(pane)}>새 대화</button
      >
      <button
        class="ghost"
        title="채팅을 끝내고 터미널로 돌아가기 (Ctrl+Shift+M). 다시 열면 같은 대화를 이어 갑니다."
        onmousedown={(e) => e.preventDefault()}
        onclick={() => void closeChat(pane)}>터미널로</button
      >
      {#if tools}{@render tools(true)}{/if}
    </header>

    <div class="body" bind:clientWidth={bodyWidth} style:--chat-bottom-h="{bottomHeight}px">
    <div class="main">
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
    <div
      class="scroller"
      class:hidden={covers}
      bind:this={scroller}
      onscroll={onScroll}
      onwheel={(e) => {
        // Already at the top, so no scroll event comes: the wheel asks for more.
        if (e.deltaY < 0) nearTop();
      }}
      onclick={onLinkClick}
      onpointerdown={(e) => {
        if (e.button !== 0) return;
        pressing = true;
        window.addEventListener(
          "pointerup",
          () => {
            pressing = false;
            // Catch up with what arrived meanwhile, unless text was picked.
            if (stick && scroller && !selectedText()) scroller.scrollTop = scroller.scrollHeight;
          },
          { once: true },
        );
      }}
      onauxclick={(e) => {
        if ((e.target as HTMLElement | null)?.closest?.("a")) e.preventDefault();
      }}
    >
      <div class="list" bind:this={list}>
        {#if moreAbove}
          <div class="more-above">위로 올리면 이전 대화를 더 불러옵니다</div>
        {/if}
        {#if s?.history && historyDrawn}
          <!-- The conversation this chat took over from the terminal's Claude
               (same session): drawn from its transcript, read-only. -->
          {@const h = s.history}
          <div class="history">
            <div class="history-head">
              터미널에서 하던 대화
              {#if h.state === "loading"}<span class="muted">(불러오는 중…)</span>{/if}
            </div>
            {#if h.state === "error"}
              <div class="muted small">이전 대화를 불러오지 못했습니다 ({h.error}). Claude 는 이 대화를 기억합니다.</div>
            {/if}
            {#if histFirst === 0}
              {#if h.compacted}
                <div class="muted small">그보다 앞의 대화는 Claude 가 요약해 두어서 여기에는 보이지 않습니다.</div>
              {/if}
              {#if h.omitted > 0}
                <div class="muted small">앞의 {h.omitted}개 항목은 생략했습니다.</div>
              {/if}
            {/if}
            {#each histRows.slice(histFirst) as r, j (histFirst + j)}
              {@const i = histFirst + j}
              {#if r.kind === "user"}
                <div class="user">{r.text}</div>
              {:else if r.kind === "text"}
                <div class="md" use:diagrams={true}>{@html renderMarkdown(r.text)}</div>
              {:else if r.kind === "tool"}
                {@const t = r.tool}
                {@const diff = stepDiff(t)}
                <div class="tool" class:err={t.err}>
                  <div class="tool-head">
                    <span class="tool-state">{#if !t.done}■{:else if t.err}✗{:else}✓{/if}</span>
                    <span class="tool-name">{t.name || "도구"}</span>
                    <span class="tool-sum">{toolSummary(t.name, t.input, cwd)}</span>
                  </div>
                  {#if diff && diff.length}
                    {@const all = historyOpen[i] === true}
                    <div class="hdiff">
                      {#each all ? diff : diff.slice(0, 24) as l, j (j)}
                        {#if l.kind === "gap"}
                          <div class="hgap">⋯</div>
                        {:else}
                          <div class="hdl {l.kind}">
                            <span class="hsign">{l.kind === "add" ? "+" : l.kind === "del" ? "-" : ""}</span><span
                              class="htext">{l.text}</span>
                          </div>
                        {/if}
                      {/each}
                      {#if diff.length > 24}
                        <button class="hmore" onclick={() => (historyOpen[i] = !all)}>
                          {all ? "줄이기" : `${diff.length - 24}줄 더 보기`}
                        </button>
                      {/if}
                    </div>
                  {/if}
                  {#if t.err && t.result}
                    <pre class="result err">{clip(t.result, 2000)}</pre>
                  {/if}
                </div>
              {/if}
            {/each}
            <div class="history-end">여기부터 채팅에서 이어집니다</div>
          </div>
        {/if}
        {#if m.items.length === 0 && !s?.history}
          <div class="empty">
            <div class="empty-title">Claude 채팅</div>
            <div>터미널 대신 대화 화면으로 Claude Code 를 씁니다. 도구 실행은 줄로, 승인 요청과 질문은 카드로 보입니다.</div>
            {#if cwd}<div class="muted">작업 폴더: {shortPath(cwd)}</div>{/if}
            <div class="muted">Enter 로 보내고 Shift+Enter 로 줄을 바꿉니다. 작업 중에는 Esc 로 멈춥니다.</div>
          </div>
        {/if}
        {#each shownItems as item (item.id)}
          {#if item.kind === "user"}
            <div class="user">{item.text}</div>
          {:else if item.kind === "assistant"}
            {#if item.text.trim() || item.streaming}
              <div class="md" class:streaming={item.streaming} use:diagrams={item.final && !item.streaming}>
                {@html renderMarkdown(item.text)}
              </div>
            {/if}
          {:else if item.kind === "thinking"}
            <details class="thinking">
              <summary>{item.streaming ? "생각하는 중…" : "생각"}</summary>
              {#if item.text.trim()}<div class="thinking-text">{item.text}</div>{/if}
            </details>
          {:else if item.kind === "tool" && EDIT_TOOLS.has(item.name)}
            <EditCard {item} base={cwd} compact={panelVisible && !item.awaiting} />
          {:else if item.kind === "tool"}
            <div class="tool" class:err={item.isError} class:awaiting={item.awaiting}>
              <button class="tool-head" onclick={() => toggle(item.id)} title="눌러서 입력과 결과 보기">
                <span class="tool-state">
                  {#if !item.done}
                    {#if item.awaiting}<span class="pause">⏸</span>{:else}<span class="spin"></span>{/if}
                  {:else if item.isError}✗{:else}✓{/if}
                </span>
                <span class="tool-name">{item.name || "도구"}</span>
                <span class="tool-sum"
                  >{toolSummary(item.name, item.input, cwd) || (item.inputJson ? "입력 작성 중…" : "")}</span
                >
                {#if item.subSteps > 0}
                  <span class="tool-sub">하위 {item.subSteps}단계{item.subLast ? `, ${item.subLast}` : ""}</span>
                {/if}
              </button>
              {#if expanded[item.id]}
                <div class="tool-body">
                  {@render toolInput(item.name, item.input)}
                  {#if item.result != null}
                    <div class="muted small">결과</div>
                    <pre class="result" class:err={item.isError}>{clip(item.result, 8000) || "(빈 결과)"}</pre>
                  {/if}
                </div>
              {/if}
            </div>
          {:else if item.kind === "permission"}
            <div class="card perm" class:settled={item.state !== "pending"}>
              <div class="card-title">{item.displayName || item.toolName} 실행 승인 요청</div>
              {#if item.description && !SELF_DESCRIBED.has(item.toolName)}
                <div class="muted">{item.description}</div>
              {/if}
              {@render toolInput(item.toolName, item.input)}
              {#if item.state === "pending"}
                <div class="actions">
                  <button class="primary" onclick={() => choose(item.id, "allow")}>허용</button>
                  {#if item.suggestions.length > 0}
                    <button onclick={() => choose(item.id, "allowRemember")}>{suggestionLabel(item.suggestions)}</button>
                  {/if}
                  <button class="danger" onclick={() => choose(item.id, "deny")}>거부</button>
                </div>
              {:else}
                <div class="settled-label {item.state}">
                  {item.state === "allowed" ? "허용함" : item.state === "denied" ? "거부함" : "취소됨"}
                </div>
              {/if}
            </div>
          {:else if item.kind === "question"}
            <div class="card question" class:settled={item.state !== "pending"}>
              <div class="card-title">Claude 의 질문</div>
              {#each item.questions as q, qi (qi)}
                <div class="q">
                  <div class="q-text">
                    {#if q.header}<span class="chip">{q.header}</span>{/if}{q.question}
                  </div>
                  {#if item.state === "pending"}
                    <div class="q-options">
                      {#each q.options as opt (opt.label)}
                        <button
                          class="opt"
                          class:picked={(picks[qkey(item.id, qi)] ?? []).includes(opt.label)}
                          onclick={() => pick(item.id, qi, opt.label, q.multiSelect)}
                        >
                          <span class="opt-label">{opt.label}</span>
                          {#if opt.description}<span class="opt-desc">{opt.description}</span>{/if}
                        </button>
                      {/each}
                      <input
                        class="other"
                        placeholder="직접 입력"
                        value={others[qkey(item.id, qi)] ?? ""}
                        oninput={(e) => (others[qkey(item.id, qi)] = e.currentTarget.value)}
                      />
                    </div>
                  {:else if item.answers}
                    <div class="answer">답: {item.answers[q.question] ?? ""}</div>
                  {/if}
                </div>
              {/each}
              {#if item.state === "pending"}
                <div class="actions">
                  <button
                    class="primary"
                    disabled={!allAnswered(item.id, item.questions)}
                    onclick={() => submitQuestions(item.id, item.questions)}>답하기</button
                  >
                  <button onclick={() => void answerQuestion(pane, item.id, null)}>건너뛰기</button>
                </div>
              {:else if item.state !== "answered"}
                <div class="settled-label">{item.state === "skipped" ? "건너뜀" : "취소됨"}</div>
              {/if}
            </div>
          {:else if item.kind === "turn"}
            <div class="turn" class:bad={!item.ok && !item.interrupted}><span>{turnLabel(item)}</span></div>
          {:else if item.kind === "notice"}
            <div class="notice {item.tone}">{item.text}</div>
          {/if}
        {/each}
        {#if status === "working" && !streamingText}
          <div class="working"><span class="spin"></span>Claude 가 작업 중입니다</div>
        {:else if status === "starting"}
          <div class="working"><span class="spin"></span>Claude 를 시작하는 중입니다</div>
        {/if}
      </div>
    </div>

    <div class="bottom" bind:clientHeight={bottomHeight}>
    {#if status === "exited"}
      <div class="exited-bar">
        Claude 세션이 끝났습니다.
        <button onclick={() => void restartChat(pane)}>이어서 다시 시작</button>
      </div>
    {/if}
    <div class="composer">
      {#if slashOpen}
        <div class="slash-menu" role="listbox" aria-label="명령과 스킬" bind:this={slashList}>
          {#each slashMatches as c, i (c.name)}
            <button
              type="button"
              role="option"
              aria-selected={i === slashIndex}
              class:picked={i === slashIndex}
              onmousedown={(e) => e.preventDefault()}
              onmousemove={() => (slashIndex = i)}
              onclick={() => pickSlash(c)}
            >
              <span class="slash-name">/{c.name}</span>
              {#if c.argumentHint}<span class="slash-hint">{c.argumentHint}</span>{/if}
              <span class="slash-desc">{c.description}</span>
            </button>
          {/each}
        </div>
      {/if}
      <textarea
        bind:this={input}
        bind:value={s.draft}
        rows="1"
        placeholder={status === "exited"
          ? "보내면 Claude 를 다시 시작해 이어 갑니다"
          : busy
            ? "작업 중에도 보낼 수 있습니다 (지금 차례가 끝나면 이어서 처리)"
            : "Claude 에게 메시지 보내기 (/ 명령과 스킬, ↑ 이전 프롬프트)"}
        onkeydown={onKey}
        oninput={() => (recall = null)}
        oncompositionstart={() => (composing = true)}
        oncompositionend={() => {
          composing = false;
          autosize();
        }}
        onblur={() => {
          // Leaving mid-composition may never deliver compositionend.
          if (!composing) return;
          composing = false;
          autosize();
        }}
      ></textarea>
      {#if busy}
        <button class="stop" title="중지 (Esc)" onclick={() => interrupt(pane)} disabled={m.interrupting}
          >{m.interrupting ? "중지 중" : "중지"}</button
        >
      {/if}
      <button class="send primary" disabled={!s.draft.trim()} onclick={() => void submit()}>보내기</button>
    </div>
    </div>
    </div>
    <!-- Always mounted, so it notices file changes while closed; shown when
         open (beside the conversation, or over it in a narrow pane). -->
    <!-- Not here while it is in the pop-out window (which runs its own). -->
    {#if !popped}
      <FilesPanel
        model={m}
        {cwd}
        visible={panelVisible && onScreen}
        overlay={covers}
        onwant={wantPanel}
        onclose={() => setPanel("closed")}
        persistKey="chat.v2"
        defaultShare={PANEL_SHARE}
        minRest={MIN_CONVERSATION}
        elsewhere={() => editedBesides(m)}
        actions={[
          { icon: "⧉", title: "파일 창을 팝업 창으로 빼기 (여러 세션을 한 창에 모아 봅니다)", run: () => popOut(pane) },
        ]}
      />
    {/if}
    </div>
  </div>
{/if}

<style>
  .chat {
    position: absolute;
    inset: 0;
    z-index: 4;
    display: flex;
    flex-direction: column;
    background: var(--bg);
    color: var(--text);
    font-family: system-ui, sans-serif;
    font-size: var(--chat-fs, 14px);
    line-height: 1.55;
  }
  header {
    flex: 0 0 auto;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    /* Room on the left for the pane's notification dot. */
    padding: 5px 8px 5px 20px;
    border-bottom: 1px solid var(--border);
    background: var(--surface-2);
    font-size: 0.82em;
  }
  .status {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-weight: 600;
    white-space: nowrap;
  }
  .status .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--muted);
  }
  .status.idle .dot {
    background: var(--green);
  }
  .status.working .dot {
    background: var(--info);
    animation: pulse 1.2s ease-in-out infinite;
  }
  .status.waiting .dot {
    background: var(--yellow);
    animation: pulse 0.9s ease-in-out infinite;
  }
  .status.exited .dot {
    background: var(--red);
  }
  @keyframes pulse {
    50% {
      opacity: 0.35;
    }
  }
  /* Takes the leftover width and gives it up first: a zero basis keeps a long
     path from pushing the controls onto a second row. */
  .cwd {
    flex: 1 1 0;
    min-width: 0;
    color: var(--muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  button {
    font: inherit;
    color: var(--text);
    background: var(--surface-3, var(--surface-2));
    border: 1px solid var(--border-2);
    border-radius: 5px;
    padding: 2px 8px;
    cursor: pointer;
  }
  button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  button.ghost {
    background: none;
  }
  button:hover:not(:disabled) {
    border-color: var(--accent);
  }
  button.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--bg);
    font-weight: 600;
  }
  button.danger {
    color: var(--red);
    border-color: color-mix(in srgb, var(--red) 60%, transparent);
    background: none;
  }

  .body {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    position: relative;
  }
  .bottom {
    flex: 0 0 auto;
  }
  .main {
    flex: 1 1 0;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .scroller.hidden {
    display: none;
  }
  .files-btn.on {
    border-color: var(--accent);
    color: var(--accent);
  }
  .files-btn.busy {
    animation: pulse 1.2s ease-in-out infinite;
  }
  .scroller {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    overflow-x: hidden;
    /* The app is user-select: none; the conversation must be copyable. */
    user-select: text;
    -webkit-user-select: text;
  }
  .scroller :global(button),
  .scroller :global(summary) {
    user-select: none;
    -webkit-user-select: none;
  }
  .list {
    display: flex;
    flex-direction: column;
    gap: 8px;
    /* The whole width of the pane: no narrow column with empty sides. */
    padding: 12px 14px 16px;
  }
  .empty {
    margin: 18px 0;
    padding: 14px;
    border: 1px dashed var(--border-2);
    border-radius: 8px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .empty-title {
    font-weight: 700;
    font-size: 1.05em;
  }
  .muted {
    color: var(--muted);
  }
  .small {
    font-size: 0.85em;
  }
  .more-above {
    align-self: center;
    padding: 2px 10px;
    font-size: 0.78em;
    color: var(--muted);
    user-select: none;
    -webkit-user-select: none;
  }
  .history {
    display: flex;
    flex-direction: column;
    gap: inherit;
    opacity: 0.92;
  }
  .history-head,
  .history-end {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--muted);
    font-size: 0.85em;
  }
  .history-head::after,
  .history-end::before,
  .history-end::after {
    content: "";
    flex: 1 1 auto;
    border-top: 1px dashed var(--border-2);
  }
  .history .tool-head {
    cursor: default;
  }
  .hdiff {
    margin: 0 8px 6px 2em;
    font-family: ui-monospace, "JetBrains Mono", "D2Coding", monospace;
    font-size: 0.84em;
    line-height: 1.4;
    background: var(--surface-2);
    border-radius: 4px;
    overflow-x: auto;
  }
  .hdl {
    display: grid;
    grid-template-columns: 1.2em minmax(0, 1fr);
    min-height: 1.4em;
  }
  .hdl.add {
    background: color-mix(in srgb, var(--green) 15%, transparent);
  }
  .hdl.del {
    background: color-mix(in srgb, var(--red) 15%, transparent);
  }
  .hsign {
    text-align: center;
    user-select: none;
    color: var(--muted);
  }
  .hdl.add .hsign {
    color: var(--green);
  }
  .hdl.del .hsign {
    color: var(--red);
  }
  .htext {
    white-space: pre-wrap;
    word-break: break-all;
    padding-right: 6px;
  }
  .hgap {
    color: var(--muted);
    text-align: center;
    font-size: 0.85em;
    border-top: 1px dashed var(--border);
  }
  .hmore {
    display: block;
    width: 100%;
    font: inherit;
    color: var(--info);
    background: none;
    border: none;
    padding: 3px;
    cursor: pointer;
  }
  .user {
    align-self: flex-end;
    max-width: 85%;
    white-space: pre-wrap;
    word-break: break-word;
    padding: 7px 11px;
    border-radius: 10px 10px 2px 10px;
    background: color-mix(in srgb, var(--accent) 22%, var(--surface-2));
    border: 1px solid color-mix(in srgb, var(--accent) 35%, transparent);
  }
  .md {
    word-break: break-word;
  }
  .md :global(p) {
    margin: 0.35em 0;
  }
  .md :global(:is(h1, h2, h3, h4)) {
    margin: 0.7em 0 0.3em;
    line-height: 1.3;
  }
  .md :global(h1) {
    font-size: 1.3em;
  }
  .md :global(h2) {
    font-size: 1.18em;
  }
  .md :global(h3) {
    font-size: 1.06em;
  }
  .md :global(:is(ul, ol)) {
    margin: 0.3em 0;
    padding-left: 1.4em;
  }
  .md :global(code) {
    font-family: ui-monospace, "JetBrains Mono", "D2Coding", monospace;
    font-size: 0.9em;
    background: var(--surface-2);
    border-radius: 4px;
    padding: 0.05em 0.35em;
  }
  .md :global(pre) {
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 8px 10px;
    overflow-x: auto;
  }
  .md :global(pre code) {
    background: none;
    padding: 0;
  }
  .md :global(table) {
    border-collapse: collapse;
    display: block;
    overflow-x: auto;
  }
  .md :global(:is(th, td)) {
    border: 1px solid var(--border-2);
    padding: 3px 8px;
  }
  .md :global(th) {
    background: var(--surface-2);
  }
  .md :global(blockquote) {
    margin: 0.4em 0;
    padding-left: 10px;
    border-left: 3px solid var(--border-2);
    color: var(--text-2, var(--muted));
  }
  .md :global(a) {
    color: var(--info);
  }
  .md :global(.mermaid-block.drawn) {
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 8px;
    overflow-x: auto;
    text-align: center;
  }
  .md :global(.mermaid-block.drawn svg) {
    max-width: 100%;
    height: auto;
  }
  .md :global(.svg-block.drawn) {
    background: #fff;
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 8px;
    text-align: center;
  }
  .md :global(.svg-block.drawn img) {
    max-width: 100%;
    max-height: 560px;
    height: auto;
    object-fit: contain;
  }
  .md :global(.mermaid-error) {
    color: var(--muted);
    font-size: 0.85em;
  }
  .md.streaming::after {
    content: "▍";
    color: var(--info);
    animation: pulse 1s steps(2) infinite;
  }

  .thinking {
    color: var(--muted);
    font-size: 0.9em;
  }
  .thinking summary {
    cursor: pointer;
  }
  .thinking-text {
    white-space: pre-wrap;
    border-left: 2px solid var(--border-2);
    padding-left: 8px;
    margin-top: 4px;
  }

  .tool {
    border: 1px solid var(--border);
    border-radius: 6px;
    background: color-mix(in srgb, var(--surface-2) 60%, transparent);
    font-size: 0.9em;
  }
  .tool.awaiting {
    border-color: var(--yellow);
  }
  .tool.err {
    border-color: color-mix(in srgb, var(--red) 50%, transparent);
  }
  .tool-head {
    display: flex;
    align-items: baseline;
    gap: 8px;
    width: 100%;
    text-align: left;
    background: none;
    border: none;
    border-radius: 6px;
    padding: 4px 8px;
  }
  .tool-state {
    flex: 0 0 auto;
    width: 1em;
    color: var(--green);
  }
  .tool.err .tool-state {
    color: var(--red);
  }
  .tool-name {
    flex: 0 0 auto;
    font-weight: 600;
  }
  .tool-sum {
    flex: 1 1 auto;
    min-width: 0;
    color: var(--text-2, var(--muted));
    font-family: ui-monospace, "JetBrains Mono", "D2Coding", monospace;
    font-size: 0.92em;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tool-sub {
    flex: 0 1 auto;
    color: var(--muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 40%;
  }
  .tool-body {
    padding: 2px 10px 8px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  pre {
    margin: 0;
    white-space: pre-wrap;
    word-break: break-word;
    font-family: ui-monospace, "JetBrains Mono", "D2Coding", monospace;
    font-size: 0.88em;
    max-height: 340px;
    overflow: auto;
    background: var(--surface-2);
    border-radius: 4px;
    padding: 6px 8px;
  }
  pre.cmd {
    color: var(--text);
  }
  pre.del {
    background: color-mix(in srgb, var(--red) 13%, var(--surface-2));
  }
  pre.add {
    background: color-mix(in srgb, var(--green) 13%, var(--surface-2));
  }
  pre.result.err {
    color: var(--red);
  }
  .file {
    font-family: ui-monospace, "JetBrains Mono", "D2Coding", monospace;
    font-size: 0.88em;
    color: var(--muted);
  }
  .spin {
    display: inline-block;
    width: 0.75em;
    height: 0.75em;
    border: 2px solid var(--info);
    border-right-color: transparent;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
    vertical-align: -0.05em;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .pause {
    color: var(--yellow);
  }

  .card {
    border: 1px solid var(--yellow);
    border-radius: 8px;
    padding: 9px 11px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    background: color-mix(in srgb, var(--yellow) 7%, var(--bg));
  }
  .card.question {
    border-color: var(--info);
    background: color-mix(in srgb, var(--info) 7%, var(--bg));
  }
  .card.settled {
    border-color: var(--border);
    background: none;
    opacity: 0.8;
  }
  .card-title {
    font-weight: 700;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 2px;
  }
  .settled-label {
    font-size: 0.88em;
    color: var(--muted);
  }
  .settled-label.allowed {
    color: var(--green);
  }
  .settled-label.denied {
    color: var(--red);
  }
  .q {
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .q-text {
    font-weight: 600;
  }
  .chip {
    display: inline-block;
    font-size: 0.78em;
    font-weight: 600;
    padding: 0 6px;
    margin-right: 6px;
    border-radius: 4px;
    background: color-mix(in srgb, var(--info) 25%, transparent);
  }
  .q-options {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .opt {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    text-align: left;
    padding: 5px 9px;
  }
  .opt.picked {
    border-color: var(--info);
    background: color-mix(in srgb, var(--info) 22%, var(--surface-2));
  }
  .opt-desc {
    font-size: 0.85em;
    color: var(--muted);
  }
  .other {
    font: inherit;
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--border-2);
    border-radius: 5px;
    padding: 4px 8px;
  }
  .answer {
    color: var(--green);
  }

  .turn {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--muted);
    font-size: 0.78em;
  }
  .turn::before,
  .turn::after {
    content: "";
    flex: 1 1 auto;
    border-top: 1px solid var(--border);
  }
  .turn.bad {
    color: var(--red);
  }
  .notice {
    white-space: pre-wrap;
    font-size: 0.86em;
    color: var(--muted);
    text-align: center;
  }
  .notice.error {
    color: var(--red);
    text-align: left;
    border: 1px solid color-mix(in srgb, var(--red) 40%, transparent);
    border-radius: 6px;
    padding: 6px 9px;
  }
  .working {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--muted);
    font-size: 0.88em;
  }

  .exited-bar {
    flex: 0 0 auto;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 10px;
    font-size: 0.86em;
    color: var(--muted);
    border-top: 1px solid var(--border);
  }
  .composer {
    position: relative;
    flex: 0 0 auto;
    display: flex;
    align-items: flex-end;
    gap: 6px;
    padding: 8px;
    border-top: 1px solid var(--border);
    background: var(--surface-2);
  }
  /* The `/` menu opens upward over the conversation, as wide as the composer. */
  .slash-menu {
    position: absolute;
    left: 8px;
    right: 8px;
    bottom: calc(100% + 4px);
    z-index: 20;
    max-height: 280px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    padding: 4px;
    background: var(--surface-2);
    border: 1px solid var(--border-2);
    border-radius: 8px;
    box-shadow: 0 6px 20px rgba(0, 0, 0, 0.35);
  }
  .slash-menu button {
    flex: 0 0 auto;
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
    padding: 5px 8px;
    text-align: left;
    font: inherit;
    color: inherit;
    background: none;
    border: none;
    border-radius: 5px;
    cursor: pointer;
  }
  .slash-menu button.picked {
    background: color-mix(in srgb, var(--accent) 22%, transparent);
  }
  .slash-name {
    flex: 0 0 auto;
    font-family: ui-monospace, "JetBrains Mono", "D2Coding", monospace;
    font-weight: 600;
  }
  .slash-hint {
    flex: 0 0 auto;
    color: var(--muted);
    font-size: 0.85em;
  }
  .slash-desc {
    flex: 1 1 auto;
    min-width: 0;
    color: var(--muted);
    font-size: 0.85em;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .composer textarea {
    flex: 1 1 auto;
    min-width: 0;
    resize: none;
    font: inherit;
    line-height: 1.45;
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--border-2);
    border-radius: 6px;
    padding: 6px 9px;
    max-height: 220px;
  }
  .composer textarea:focus {
    outline: none;
    border-color: var(--accent);
  }
  .composer .stop {
    color: var(--red);
    border-color: color-mix(in srgb, var(--red) 60%, transparent);
  }
</style>
