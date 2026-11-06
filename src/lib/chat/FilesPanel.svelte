<script lang="ts">
  // The files panel: a read-only code editor next to the conversation that
  // shows Claude's file changes while they happen.
  // - Edit / Write: as soon as the file of the call is known (its input is
  //   still streaming), the file opens in a tab, scrolls to the place, marks
  //   the text being replaced in red and types the new text in green as it
  //   arrives, paced so it can be followed. Then the red text goes.
  // - Any other change to a file (a command, a script, a formatter, another
  //   program): once it shows on disk (git status, after each tool call and
  //   every few seconds while shown), the panel plays it back the same way,
  //   from what the file looked like before.
  // - Following: the editor goes to the file being changed. Picking another
  //   file to read pauses that; after a few seconds without touching the
  //   panel, or at the user's next message, the next change takes it back.
  // - Beside the editor: git's changed files and the folder tree.
  // The panel stays mounted while hidden, so it sees changes before it is
  // shown, and asks to be shown (`onwant`) when one starts.
  import { onDestroy, tick, untrack } from "svelte";
  import { EditorView } from "@codemirror/view";
  import type { EditorState, TransactionSpec } from "@codemirror/state";
  import {
    workHead,
    workList,
    workRead,
    workRoot,
    workStatus,
    type WorkChange,
    type WorkChangeStatus,
    type WorkEntry,
    type WorkRoot,
  } from "../ipc";
  import type { ChatItem, FileFeed } from "./reducer";
  import { diffLines } from "./lineDiff";
  import { partialFields, type PartialField } from "./partialJson";
  import { composeRegion, diffSegments, streamSegments, totalCost, type Seg } from "./liveRegion";
  import {
    languageFor,
    makeState,
    setLanguage,
    setLiveMarks,
    setPlusLines,
    plusLinesOf,
    type LiveMarks,
    type Span,
  } from "./codeView";
  import { shortPath } from "./format";

  type ToolItem = Extract<ChatItem, { kind: "tool" }>;

  let {
    model,
    cwd,
    visible,
    overlay = false,
    onwant = () => {},
    onclose,
    persistKey,
    defaultShare = 0.5,
    minRest = 360,
    fill = false,
    label = "",
    actions = [],
    closable = true,
  }: {
    model: FileFeed;
    cwd: string | null;
    /** On screen. While hidden the panel still tracks changes, without animating. */
    visible: boolean;
    /** Drawn over what it sits next to (a narrow pane) instead of beside it. */
    overlay?: boolean;
    /** A file change started: the panel would like to be shown. */
    onwant?: () => void;
    onclose: () => void;
    /** Where the dragged width is remembered ("chat", "term"). */
    persistKey: string;
    /** The panel's share of the width until the user drags its edge. */
    defaultShare?: number;
    /** Pixels the neighbour (conversation, terminal) always keeps. */
    minRest?: number;
    /** Fill the box it sits in (a cell of the pop-out window): no edge to drag. */
    fill?: boolean;
    /** Which session this is, shown first in the header (the pop-out window). */
    label?: string;
    /** Extra header buttons (pop out, put back, enlarge). */
    actions?: { icon: string; title: string; run: () => void; on?: boolean }[];
    /** Show the ✕ that closes the panel. */
    closable?: boolean;
  } = $props();

  const BADGE: Record<WorkChangeStatus, [string, string]> = {
    modified: ["M", "고침"],
    added: ["A", "추가 (git 에 올림)"],
    new: ["N", "새 파일 (git 에 아직 없음)"],
    deleted: ["D", "지움"],
    renamed: ["R", "이름 바꿈"],
    copied: ["C", "복사"],
    conflict: ["!", "충돌"],
  };
  /** An editor's save-in-progress file (written, then renamed over the real one). */
  const TEMP_FILE = /\.tmp\.\d+\.[0-9a-f]+$/;
  /** Drawing speed against the first version (the user asked for 1.5x). */
  const SPEED = 1.5;
  /** Typing pace: at least `minRate` characters a second, and never more
   *  than about `lag` seconds behind the text that has arrived. */
  const LIVE = { minRate: 60 * SPEED, lag: 0.7 / SPEED };
  const REPLAY = { minRate: 140 * SPEED, lag: 0.35 / SPEED };
  /** The panel never gets narrower than this when beside its neighbour. */
  const MIN_PANEL = 320;
  const MAX_TABS = 10;

  let dead = false;
  onDestroy(() => {
    dead = true;
  });

  // ---- the root: the git repository's top, or the chat's folder ----
  let root = $state<WorkRoot | null>(null);
  let rootError = $state<string | null>(null);
  $effect(() => {
    const dir = cwd;
    if (!dir) return;
    let alive = true;
    workRoot(dir)
      .then((r) => {
        if (!alive) return;
        root = r;
        rootError = null;
        dirs = {};
        expanded = { "": true };
        changes = [];
        known.clear();
        baselined = false;
        void refresh();
      })
      .catch((e) => {
        if (alive) rootError = String(e);
      });
    return () => {
      alive = false;
    };
  });

  const rootName = $derived(root ? (root.root.split("/").filter(Boolean).pop() ?? "/") : "");

  function relOf(abs: string | null | undefined): string | null {
    if (!abs || !root) return null;
    const base = root.root.endsWith("/") ? root.root : root.root + "/";
    return abs.startsWith(base) ? abs.slice(base.length) : null;
  }
  function absOf(rel: string): string {
    if (!root) return rel;
    return root.root.endsWith("/") ? root.root + rel : `${root.root}/${rel}`;
  }
  const dirOf = (rel: string) => (rel.includes("/") ? rel.slice(0, rel.lastIndexOf("/") + 1) : "");
  const baseOf = (p: string) => p.slice(p.lastIndexOf("/") + 1);

  // ---- changed files and the folder tree ----
  type Level = WorkEntry[] | { error: string };
  let changes = $state<WorkChange[]>([]);
  let dirs = $state<Record<string, Level>>({});
  let expanded = $state<Record<string, boolean>>({ "": true });
  const changeOf = $derived(new Map(changes.map((c) => [c.path, c])));
  const changedDirs = $derived.by(() => {
    const set = new Set<string>();
    for (const c of changes) {
      const parts = c.path.split("/");
      for (let i = 1; i < parts.length; i++) set.add(parts.slice(0, i).join("/"));
    }
    return set;
  });

  async function loadDir(rel: string) {
    const r = root;
    if (!r) return;
    let level: Level;
    try {
      level = (await workList(r.root, rel, r.git)).filter((e) => !TEMP_FILE.test(e.name));
    } catch (e) {
      level = { error: String(e) };
    }
    if (root === r) dirs[rel] = level;
  }

  function toggleDir(rel: string) {
    expanded[rel] = !expanded[rel];
    if (expanded[rel] && !dirs[rel]) void loadDir(rel);
  }

  // ---- what each file looked like on disk, last time we looked ----
  // Seeded with the files already changed when the chat opened, so changes
  // made before it are never played back as if they were new.
  const known = new Map<string, string | null>();
  let baselined = false;

  async function readText(r: WorkRoot, rel: string): Promise<string | null> {
    try {
      const f = await workRead(r.root, rel);
      return f.kind === "text" && !f.truncated ? f.text : null;
    } catch {
      return null;
    }
  }
  async function headText(r: WorkRoot, rel: string): Promise<string | null> {
    if (!r.git) return null;
    try {
      const f = await workHead(r.root, rel);
      return f.kind === "text" && !f.truncated ? f.text : f.kind === "missing" ? "" : null;
    } catch {
      return null;
    }
  }

  // ---- where each file started: "+" marks the lines added since ----
  // What the panel first saw of a file (for a file that was clean, the last
  // commit's content), so only changes made while it watched get a "+".
  const origin = new Map<string, string>();
  function noteOrigin(path: string, text: string | null | undefined) {
    if (typeof text === "string" && !origin.has(path)) origin.set(path, text);
  }
  function updatePlus(path: string) {
    const st = stateOf(path);
    if (!st) return;
    const base = origin.get(path);
    const doc = st.doc.toString();
    const lines: number[] = [];
    if (base !== undefined && base !== doc) {
      for (const h of diffLines(base.split("\n"), doc.split("\n"))) {
        for (let b = h.bStart; b < h.bEnd; b++) lines.push(b + 1);
      }
    }
    apply(path, { effects: setPlusLines.of(lines) });
  }

  /** Files whose disk content moved away from what we knew: play them back. */
  async function detectChanges(r: WorkRoot, list: WorkChange[]) {
    const status = new Map(list.map((c) => [c.path, c]));
    const rels = new Set<string>([...status.keys(), ...known.keys()]);
    for (const t of tabs) if (t.rel) rels.add(t.rel);
    const first = !baselined;
    let replays = 0;
    for (const rel of [...rels].slice(0, 60)) {
      if (dead || root !== r) return;
      const abs = absOf(rel);
      if (busy.has(abs) || TEMP_FILE.test(rel)) continue;
      const st = status.get(rel);
      const cur = await readText(r, rel);
      if (first) {
        known.set(rel, cur);
        noteOrigin(abs, cur);
        continue;
      }
      let before = known.get(rel);
      if (before === undefined) {
        // Clean when we last looked: it was what the last commit has.
        before = st?.status === "new" ? "" : await headText(r, rel);
      }
      known.set(rel, cur);
      // Before the replay below opens the file (and reads the new content).
      noteOrigin(abs, before);
      // Clean in git again (committed): the "+" marks start over from here.
      if (r.git && !st && cur !== null && origin.get(abs) !== cur) {
        origin.set(abs, cur);
        if (stateOf(abs)) updatePlus(abs);
      }
      if (cur === null || cur === before) continue;
      // If the file is open, what the user sees is the real "before".
      const from = stateOf(abs) ? docOf(abs) : before;
      if (from === null || from === cur) continue;
      if (replays++ >= 8) {
        if (stateOf(abs)) {
          setDoc(abs, cur);
          updatePlus(abs);
        }
        continue;
      }
      enqueue({ kind: "replay", path: abs, before: from, after: cur });
    }
    baselined = true;
  }

  // ---- refresh: never two at once; a request during one runs once after ----
  let inflight = false;
  let again = false;
  let lastRefresh = 0;
  let lastCost = 0;
  async function refresh() {
    const r = root;
    if (!r || dead) return;
    if (inflight) {
      again = true;
      return;
    }
    inflight = true;
    const started = performance.now();
    try {
      let list: WorkChange[] = changes;
      if (r.git) {
        const next = await workStatus(r.root).catch(() => null);
        // Without git's answer we cannot tell old changes from new ones:
        // compare nothing this time (the first look would take a wrong baseline).
        if (!next || root !== r) return;
        changes = next.filter((c) => !TEMP_FILE.test(c.path));
        list = changes;
      }
      if (visible) {
        const open = Object.keys(expanded).filter((k) => expanded[k]);
        await Promise.all(open.map((rel) => loadDir(rel)));
      }
      await detectChanges(r, list);
    } finally {
      inflight = false;
      lastRefresh = Date.now();
      lastCost = performance.now() - started;
      if (again) {
        again = false;
        void refresh();
      }
    }
  }

  let timer: ReturnType<typeof setTimeout> | null = null;
  function schedule(ms: number) {
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => {
      timer = null;
      void refresh();
    }, ms);
  }
  // A tool call finished (or a turn ended): something on disk may differ.
  $effect(() => {
    void model.fileSeq;
    untrack(() => schedule(120));
  });
  // While shown: changes made outside the chat, or by a command still running.
  $effect(() => {
    if (!visible) return;
    const id = setInterval(() => {
      if (document.hidden) return;
      const busyChat = model.status === "working" || model.status === "waiting";
      const gap = Math.max(busyChat ? 3000 : 15000, lastCost * 10);
      if (Date.now() - lastRefresh < gap) return;
      void refresh();
    }, 1000);
    return () => clearInterval(id);
  });
  onDestroy(() => {
    if (timer) clearTimeout(timer);
  });

  // ---- tabs and the editor ----
  interface Tab {
    path: string;
    rel: string | null;
    name: string;
    note: string | null;
  }
  let tabs = $state<Tab[]>([]);
  let active = $state<string | null>(null);
  /** Editor state of each tab; the active tab's lives in the view. */
  const states = new Map<string, EditorState>();
  let view: EditorView | null = null;
  let host = $state<HTMLElement>();
  let follow = $state(true);
  /** When the user last picked a file to read (which paused following), or
   *  0 when they turned following off with its checkbox. */
  let peekAt = 0;
  /** This long without touching the panel, and the next change takes the
   *  editor back to the file being changed. */
  const RESUME_MS = 4000;
  function peek() {
    follow = false;
    peekAt = Date.now();
  }
  function setFollow(on: boolean) {
    follow = on;
    peekAt = 0;
    // Following again jumps to what is being changed now.
    if (on && activity && tabs.some((t) => t.path === activity!.path)) activate(activity.path);
  }
  // Reading another file (scrolling, clicking in it) keeps the pause going.
  let panelEl = $state<HTMLElement>();
  $effect(() => {
    const el = panelEl;
    if (!el) return;
    const touch = () => {
      if (!follow && peekAt) peekAt = Date.now();
    };
    // The panel's buttons and boxes must not take the keyboard away from the
    // conversation's input or the terminal next to the panel.
    const keepFocus = (ev: MouseEvent) => {
      if ((ev.target as HTMLElement | null)?.closest?.("button, label, input")) ev.preventDefault();
    };
    el.addEventListener("pointerdown", touch, true);
    el.addEventListener("wheel", touch, { capture: true, passive: true });
    el.addEventListener("mousedown", keepFocus, true);
    return () => {
      el.removeEventListener("pointerdown", touch, true);
      el.removeEventListener("wheel", touch, true);
      el.removeEventListener("mousedown", keepFocus, true);
    };
  });

  // ---- width: a share of the space the panel sits in, set by dragging its
  // left edge. While dragging only a guide line moves; the size changes once,
  // on release (a terminal beside the panel would otherwise resize the program
  // in it on every pixel).
  const shareKey = untrack(() => `amux.files.share.${persistKey}`);
  function loadShare(): number {
    try {
      const v = Number(localStorage.getItem(shareKey));
      if (v > 0.05 && v < 0.95) return v;
    } catch {
      /* storage unavailable: the default */
    }
    return untrack(() => defaultShare);
  }
  let share = $state(loadShare());
  let guideX = $state<number | null>(null);
  let guideTop = $state(0);
  let guideHeight = $state(0);
  /** The box the panel takes its share of (a wrapper without a box of its
   *  own, like the terminal's `display: contents` one, is skipped). */
  function container(): HTMLElement | null {
    let el = panelEl?.parentElement ?? null;
    while (el && getComputedStyle(el).display === "contents") el = el.parentElement;
    return el;
  }
  function gripDown(e: PointerEvent) {
    const box = container()?.getBoundingClientRect();
    if (e.button !== 0 || !box || box.width <= 0) return;
    e.preventDefault();
    const grip = e.currentTarget as HTMLElement;
    grip.setPointerCapture(e.pointerId);
    guideTop = box.top;
    guideHeight = box.height;
    const lo = box.left + minRest;
    const hi = box.right - MIN_PANEL;
    const clampX = (x: number) => (lo > hi ? hi : Math.max(lo, Math.min(hi, x)));
    guideX = clampX(e.clientX);
    const move = (ev: PointerEvent) => {
      guideX = clampX(ev.clientX);
    };
    const finish = (apply: boolean) => {
      grip.removeEventListener("pointermove", move);
      grip.removeEventListener("pointerup", up);
      grip.removeEventListener("pointercancel", cancel);
      if (apply && guideX !== null) {
        share = (box.right - guideX) / box.width;
        try {
          localStorage.setItem(shareKey, String(share));
        } catch {
          /* applied, just not remembered */
        }
      }
      guideX = null;
    };
    const up = () => finish(true);
    const cancel = () => finish(false);
    grip.addEventListener("pointermove", move);
    grip.addEventListener("pointerup", up);
    grip.addEventListener("pointercancel", cancel);
  }
  function gripReset() {
    share = defaultShare;
    try {
      localStorage.removeItem(shareKey);
    } catch {
      /* nothing to forget */
    }
  }
  // A new request: show its changes again, even if following was turned off.
  $effect(() => {
    void model.userSeq;
    untrack(() => {
      follow = true;
      peekAt = 0;
    });
  });
  let panelWidth = $state(0);
  /** The list sits beside the editor when there is room, else over it. */
  const listBeside = $derived(panelWidth >= 680);
  /** The list (changed files, folders) starts closed: the editor gets the
   *  room, and ☰ 목록 opens it. It stays as the user leaves it while the
   *  panel lives. */
  let listOpen = $state(false);

  function stateOf(path: string): EditorState | null {
    if (view && active === path) return view.state;
    return states.get(path) ?? null;
  }
  function docOf(path: string): string {
    return stateOf(path)?.doc.toString() ?? "";
  }
  function apply(path: string, spec: TransactionSpec) {
    if (view && active === path) view.dispatch(spec);
    else {
      const st = states.get(path);
      if (st) states.set(path, st.update(spec).state);
    }
  }
  function setDoc(path: string, text: string) {
    const st = stateOf(path);
    if (!st) return;
    apply(path, { changes: { from: 0, to: st.doc.length, insert: text }, effects: setLiveMarks.of(null) });
  }

  function activate(path: string) {
    if (active === path) return;
    if (view && active) states.set(active, view.state);
    active = path;
    const st = states.get(path);
    if (view && st) view.setState(st);
  }

  // The view exists only while the panel is shown: an editor created in a
  // hidden box measures as zero and cannot scroll.
  $effect(() => {
    const el = host;
    if (!visible || !el) return;
    untrack(() => {
      // Opened after changes it could not show: start on the latest one.
      if (!active) {
        const ready = tabs.filter((t) => states.has(t.path));
        const pick = ready.find((t) => t.path === activity?.path) ?? ready[ready.length - 1];
        if (pick) active = pick.path;
      }
      const st = (active && states.get(active)) || makeState("");
      view = new EditorView({ state: st, parent: el });
    });
    return () => {
      if (view) {
        if (active) states.set(active, view.state);
        view.destroy();
        view = null;
      }
    };
  });

  async function ensureTab(path: string): Promise<Tab> {
    let tab = tabs.find((t) => t.path === path);
    if (!tab) {
      tabs.push({ path, rel: relOf(path), name: baseOf(path), note: null });
      tab = tabs[tabs.length - 1];
      while (tabs.length > MAX_TABS) {
        const drop = tabs.find((t) => t.path !== active && t.path !== path && !busy.has(t.path));
        if (!drop) break;
        states.delete(drop.path);
        tabs.splice(tabs.indexOf(drop), 1);
      }
    }
    if (stateOf(path)) return tab;
    const r = root;
    const rel = relOf(path);
    let text = "";
    let note: string | null = null;
    if (r && rel !== null) {
      const f = await workRead(r.root, rel).catch(() => null);
      if (!f) note = "파일을 읽지 못했습니다";
      else if (f.kind === "text") {
        text = f.text;
        if (f.truncated) note = "파일이 커서 앞부분만 보입니다";
        if (!known.has(rel) && !f.truncated) known.set(rel, f.text);
        if (!f.truncated) noteOrigin(path, f.text);
      } else if (f.kind === "missing") {
        note = "새 파일";
        noteOrigin(path, "");
      }
      else note = "바이너리 파일이라 내용을 보여 주지 않습니다";
    } else {
      note = "작업 폴더 밖의 파일이라 바뀌는 부분만 보입니다";
    }
    if (!stateOf(path)) states.set(path, makeState(text));
    updatePlus(path);
    tab.note = note;
    void languageFor(path).then((lang) => {
      if (lang && stateOf(path)) apply(path, { effects: setLanguage(lang) });
    });
    return tab;
  }

  function openFile(abs: string) {
    peek();
    if (!listBeside) listOpen = false;
    void ensureTab(abs).then(() => activate(abs));
  }

  function closeTab(path: string) {
    if (busy.has(path)) return;
    const i = tabs.findIndex((t) => t.path === path);
    if (i < 0) return;
    tabs.splice(i, 1);
    if (active === path) {
      const next = tabs[Math.min(i, tabs.length - 1)];
      if (next) activate(next.path);
      else {
        active = null;
        view?.setState(makeState(""));
      }
    }
    states.delete(path);
  }

  // ---- the playback queue ----
  type Job =
    | { kind: "edit" | "write"; path: string; row: ToolItem }
    | { kind: "replay"; path: string; before: string; after: string };
  const queue: Job[] = [];
  const busy = new Set<string>();
  let running = false;
  let activity = $state<{ path: string; kind: Job["kind"]; text: string } | null>(null);

  function enqueue(job: Job) {
    queue.push(job);
    busy.add(job.path);
    if (!tabs.some((t) => t.path === job.path)) void ensureTab(job.path);
    onwant();
    void pump();
  }

  async function pump() {
    if (running) return;
    running = true;
    try {
      while (queue.length && !dead) {
        const job = queue.shift()!;
        try {
          await play(job);
        } catch (e) {
          console.warn("[amux] live edit", e);
        } finally {
          updatePlus(job.path);
          if (!queue.some((j) => j.path === job.path)) busy.delete(job.path);
        }
      }
    } finally {
      running = false;
      schedule(150);
    }
  }

  const frame = () =>
    new Promise<void>((resolve) => {
      if (document.hidden) setTimeout(resolve, 100);
      else requestAnimationFrame(() => resolve());
    });
  const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));
  /** Wait until `get` returns something other than undefined (or 10 minutes). */
  async function until<T>(get: () => T | undefined): Promise<T | undefined> {
    const end = Date.now() + 600_000;
    for (;;) {
      const v = get();
      if (v !== undefined || dead || Date.now() > end) return v;
      await frame();
    }
  }

  /** Animate only when the user can see it: the panel is shown, and the file
   *  is the one on screen or following may bring it there. */
  async function canAnimate(path: string): Promise<boolean> {
    // The panel may be opening for this very job: give it a moment.
    for (let i = 0; i < 20 && !(visible && view); i++) await sleep(15);
    if (!visible || !view || document.hidden) return false;
    if (!follow && peekAt && Date.now() - peekAt >= RESUME_MS) follow = true;
    if (!follow && active !== path) return false;
    activate(path);
    return true;
  }
  const watching = (path: string) => visible && !!view && active === path && !document.hidden && !dead;

  // Fields of a tool call's input, while it streams and after.
  const parsed = new WeakMap<object, { len: number; fields: Record<string, PartialField> }>();
  function fieldsOf(row: ToolItem): Record<string, PartialField> {
    if (row.inputDone) {
      const out: Record<string, PartialField> = {};
      for (const [k, v] of Object.entries(row.input)) if (typeof v === "string") out[k] = { value: v, done: true };
      return out;
    }
    const raw = row.inputJson;
    const hit = parsed.get(row);
    if (hit && hit.len === raw.length) return hit.fields;
    const fields = partialFields(raw);
    parsed.set(row, { len: raw.length, fields });
    return fields;
  }

  /** Writes into one region of a file, changing only what differs from
   *  the last frame, and keeps the cursor in view. */
  function regionWriter(path: string, start: number, initial: string, zone: boolean) {
    let cur = initial;
    let lastScroll = -1;
    return {
      get text() {
        return cur;
      },
      draw(text: string, m: { del: Span[]; add: Span[]; caret: number | null; settle?: boolean } | null) {
        let a = 0;
        const max = Math.min(cur.length, text.length);
        while (a < max && cur.charCodeAt(a) === text.charCodeAt(a)) a++;
        let b = 0;
        while (b < max - a && cur.charCodeAt(cur.length - 1 - b) === text.charCodeAt(text.length - 1 - b)) b++;
        const changes =
          a === cur.length && a === text.length
            ? undefined
            : { from: start + a, to: start + cur.length - b, insert: text.slice(a, text.length - b) };
        const shift = (r: Span) => ({ from: start + r.from, to: start + r.to });
        const marks: LiveMarks | null = m
          ? {
              zone: zone ? { from: start, to: start + text.length } : null,
              del: m.del.map(shift),
              add: m.add.map(shift),
              caret: m.caret == null ? null : start + m.caret,
              settle: m.settle,
            }
          : null;
        const effects: unknown[] = [setLiveMarks.of(marks)];
        if (m?.caret != null && view && active === path) {
          // A jump (to the next change) centers it; typing just keeps it in view.
          const pos = start + m.caret;
          const line = view.state.doc.lineAt(Math.min(pos, view.state.doc.length)).number;
          const far = lastScroll < 0 || Math.abs(line - lastScroll) > 12;
          effects.push(EditorView.scrollIntoView(pos, { y: far ? "center" : "nearest", yMargin: 56 }));
          lastScroll = line;
        }
        apply(path, { changes, effects: effects as never });
        cur = text;
      },
    };
  }

  /**
   * Draws a change while it arrives: `segsNow()` describes it so far, and is
   * final once `complete()`. Paced by `pace`; finishes at once if the user
   * stops watching. Leaves the going lines struck through (see `land`).
   */
  async function animate(
    path: string,
    w: ReturnType<typeof regionWriter>,
    segsNow: () => Seg[],
    complete: () => boolean,
    pace = LIVE,
  ) {
    let budget = 0;
    let last = performance.now();
    for (;;) {
      if (!watching(path)) {
        await until(() => (complete() || dead ? true : undefined));
        const all = composeRegion(segsNow(), Infinity);
        w.draw(all.text, { del: all.del, add: all.add, caret: null });
        return;
      }
      const segs = segsNow();
      const total = totalCost(segs);
      const now = performance.now();
      const dt = Math.min(0.1, (now - last) / 1000);
      last = now;
      const backlog = total - budget;
      if (backlog > 0) budget = Math.min(total, budget + Math.max(pace.minRate, backlog / pace.lag) * dt);
      const c = composeRegion(segs, budget);
      w.draw(c.text, { del: c.del, add: c.add, caret: c.caret });
      if (complete() && budget >= total) return;
      await frame();
    }
  }

  /** The change landed: the going lines vanish, the new ones fade to normal. */
  async function land(w: ReturnType<typeof regionWriter>, segs: Seg[]) {
    await sleep(320 / SPEED);
    const fin = composeRegion(segs, Infinity, true);
    w.draw(fin.text, { del: [], add: fin.add, caret: null, settle: true });
  }

  function say(path: string, kind: Job["kind"], text: string) {
    activity = { path, kind, text };
  }

  async function play(job: Job) {
    await ensureTab(job.path);
    if (job.kind === "replay") return playReplay(job);
    // Not on screen (the panel is closed, or the user is reading another
    // file): apply the change without drawing it, so the next edits of the
    // same file still find their text.
    if (!(await canAnimate(job.path))) return applyQuietly(job);
    if (job.kind === "edit") return playEdit(job.path, job.row);
    return playWrite(job.path, job.row);
  }

  async function applyQuietly(job: Extract<Job, { kind: "edit" | "write" }>) {
    const { path, row } = job;
    say(path, job.kind, visible ? "다른 파일을 보고 있어 결과만 반영합니다" : "패널이 닫혀 있어 결과만 반영합니다");
    const ready = await until(() => (row.inputDone || row.done ? true : undefined));
    if (!ready || (row.done && row.isError) || !stateOf(path)) return;
    const f = fieldsOf(row);
    if (job.kind === "write") {
      if (f.content) setDoc(path, f.content.value);
      return;
    }
    const old = f.old_string?.value;
    if (!old) return;
    const doc = docOf(path);
    const at = doc.indexOf(old);
    // Not there: the file differs from what we know; the refresh after the
    // call sets it from disk.
    if (at < 0) return;
    const neu = f.new_string?.value ?? "";
    const all = (row.input as Record<string, unknown>).replace_all === true;
    setDoc(path, all ? doc.split(old).join(neu) : doc.slice(0, at) + neu + doc.slice(at + old.length));
  }

  async function waitForDecision(path: string, row: ToolItem, kind: Job["kind"]) {
    while (row.awaiting && !row.done && !dead) {
      say(path, kind, "승인을 기다리는 중");
      await frame();
    }
  }

  async function playEdit(path: string, row: ToolItem) {
    say(path, "edit", "바꿀 곳을 찾는 중");
    const old = await until(() => {
      const f = fieldsOf(row);
      if (f.old_string?.done) return f.old_string.value;
      return row.inputDone || row.done ? null : undefined;
    });
    if (!old) return;
    if (docOf(path).indexOf(old) < 0 && relOf(path) === null) setDoc(path, old);
    const doc = docOf(path);
    const at = doc.indexOf(old);
    const st = stateOf(path);
    if (at < 0 || !st) {
      say(path, "edit", "파일 내용이 달라져 결과로 맞춥니다");
      return;
    }
    // The whole lines the old text sits on.
    const stop = at + old.length;
    const start = st.doc.lineAt(at).from;
    const end = old.endsWith("\n") ? stop : st.doc.lineAt(stop).to;
    const prefix = doc.slice(start, at);
    const suffix = doc.slice(stop, end);
    const oldRegion = doc.slice(start, end);
    const w = regionWriter(path, start, oldRegion, true);
    w.draw(oldRegion, { del: [], add: [], caret: 0 });
    await sleep(Math.min(600, 250 + oldRegion.split("\n").length * 25) / SPEED);
    say(path, "edit", "고치는 중");
    const newDone = () => fieldsOf(row).new_string?.done === true || row.inputDone || row.done;
    const segsNow = () =>
      streamSegments(oldRegion, prefix + (fieldsOf(row).new_string?.value ?? "") + (newDone() ? suffix : ""), newDone());
    await animate(path, w, segsNow, newDone);
    await waitForDecision(path, row, "edit");
    if (row.done && row.isError) {
      w.draw(oldRegion, null);
      say(path, "edit", "적용되지 않았습니다");
      return;
    }
    await land(w, segsNow());
    say(path, "edit", row.done ? "고쳤습니다" : "고쳤습니다 (저장 대기)");
  }

  async function playWrite(path: string, row: ToolItem) {
    const existing = docOf(path);
    say(path, "write", existing ? "파일을 다시 쓰는 중" : "새 파일을 쓰는 중");
    const w = regionWriter(path, 0, existing, false);
    const done = () => fieldsOf(row).content?.done === true || row.inputDone || row.done;
    const segsNow = () => streamSegments(existing, fieldsOf(row).content?.value ?? "", done());
    await animate(path, w, segsNow, done);
    await waitForDecision(path, row, "write");
    if (row.done && row.isError) {
      w.draw(existing, null);
      say(path, "write", "적용되지 않았습니다");
      return;
    }
    await land(w, segsNow());
    say(path, "write", row.done ? "다 썼습니다" : "다 썼습니다 (저장 대기)");
  }

  async function playReplay(job: Extract<Job, { kind: "replay" }>) {
    const { path, before, after } = job;
    if (docOf(path) !== before) setDoc(path, before);
    if (!(await canAnimate(path))) {
      setDoc(path, after);
      return;
    }
    say(path, "replay", "파일에 생긴 변경을 다시 보여 주는 중");
    const segs = diffSegments(before, after);
    const w = regionWriter(path, 0, before, false);
    await animate(path, w, () => segs, () => true, REPLAY);
    await land(w, segs);
    if (docOf(path) !== after) setDoc(path, after);
    say(path, "replay", "파일에 생긴 변경을 보여 줬습니다");
  }

  // New file-changing calls from Claude, in the order their file became known.
  let seenModel: FileFeed | null = null;
  let seenEdits = 0;
  $effect(() => {
    const m = model;
    const n = m.editOrder.length;
    untrack(() => {
      if (m !== seenModel) {
        seenModel = m;
        seenEdits = 0;
      }
      for (const id of m.editOrder.slice(seenEdits, n)) {
        const at = m.toolIds[id];
        const row = at === undefined ? undefined : m.items[at];
        if (!row || row.kind !== "tool" || !row.path) continue;
        if (row.name === "Edit") enqueue({ kind: "edit", path: row.path, row });
        else if (row.name === "Write") enqueue({ kind: "write", path: row.path, row });
        // Other tools (NotebookEdit) are played back from disk afterwards.
      }
      seenEdits = n;
    });
  });

  // A file opens: in a narrow panel the list gets out of the editor's way.
  $effect(() => {
    if (active && !listBeside) untrack(() => (listOpen = false));
  });

  // For the harness only (dev builds): what the editor shows.
  if (import.meta.env.DEV) {
    const hook = {
      doc: () => (view ? view.state.doc.toString() : null),
      active: () => active,
      tabs: () => tabs.map((t) => t.path),
      busy: () => [...busy],
      plus: () => (view ? plusLinesOf(view.state) : null),
      cwd: () => cwd,
    };
    const w = window as unknown as { __amuxLive?: unknown; __amuxLives?: unknown[] };
    w.__amuxLive = hook;
    (w.__amuxLives ??= []).push(hook);
    onDestroy(() => {
      w.__amuxLives = (w.__amuxLives ?? []).filter((h) => h !== hook);
    });
  }

  // Re-measure a tab the panel was hidden over.
  $effect(() => {
    if (visible) void tick().then(() => view?.requestMeasure());
  });
</script>

{#snippet treeLevel(rel: string, depth: number)}
  {@const level = dirs[rel]}
  {#if level === undefined}
    <div class="node muted" style:padding-left="{depth * 12 + 22}px">불러오는 중…</div>
  {:else if !Array.isArray(level)}
    <div class="node err" style:padding-left="{depth * 12 + 22}px">{level.error}</div>
  {:else if level.length === 0}
    <div class="node muted" style:padding-left="{depth * 12 + 22}px">(비어 있음)</div>
  {:else}
    {#each level as e (e.name)}
      {@const child = rel ? `${rel}/${e.name}` : e.name}
      {#if e.dir}
        <button
          class="node dir"
          class:changed={changedDirs.has(child)}
          style:padding-left="{depth * 12 + 6}px"
          onclick={() => toggleDir(child)}
        >
          <span class="twisty">{expanded[child] ? "▾" : "▸"}</span>
          <span class="name">{e.name}</span>
          {#if changedDirs.has(child)}<span class="dot" title="안에 바뀐 파일이 있습니다"></span>{/if}
        </button>
        {#if expanded[child]}{@render treeLevel(child, depth + 1)}{/if}
      {:else}
        {@const ch = changeOf.get(child)}
        {@const abs = absOf(child)}
        <button
          class="node file"
          class:sel={active === abs}
          class:editing={activity?.path === abs}
          style:padding-left="{depth * 12 + 22}px"
          onclick={() => openFile(abs)}
          title={child}
        >
          <span class="name {ch ? `st-${ch.status}` : ''}">{e.name}</span>
          {#if ch}<span class="badge st-{ch.status}" title={BADGE[ch.status][1]}>{BADGE[ch.status][0]}</span>{/if}
        </button>
      {/if}
    {/each}
  {/if}
{/snippet}

<aside
  class="files"
  class:overlay
  class:fill
  class:hidden={!visible}
  style:--share={share}
  style:--min-rest="{minRest}px"
  bind:clientWidth={panelWidth}
  bind:this={panelEl}
>
  {#if !overlay && !fill}
    <button
      class="grip"
      tabindex="-1"
      aria-label="패널 너비 조절"
      title="끌어서 너비 조절 (두 번 누르면 처음 너비)"
      onpointerdown={gripDown}
      ondblclick={gripReset}
    ></button>
  {/if}
  {#if guideX !== null}
    <div class="guide" style:left="{guideX}px" style:top="{guideTop}px" style:height="{guideHeight}px"></div>
  {/if}
  <div class="phead">
    {#if label}<span class="slabel" title={label}>{label}</span>{/if}
    <button
      class="list-toggle"
      class:on={listOpen}
      title="바뀐 파일과 폴더 목록"
      onclick={() => (listOpen = !listOpen)}>☰ 목록{changes.length ? ` ${changes.length}` : ""}</button
    >
    <span class="root" title={root?.root ?? cwd ?? ""}>📂 {rootName || "작업 폴더"}</span>
    {#if root?.branch}<span class="branch" title="git 브랜치">⎇ {root.branch}</span>{/if}
    <span class="spacer"></span>
    <label class="follow" title="Claude 가 바꾸는 파일을 자동으로 보여 줍니다">
      <input type="checkbox" checked={follow} onchange={(e) => setFollow(e.currentTarget.checked)} /> 따라가기
    </label>
    {#each actions as a (a.icon)}
      <button class="icon-btn" class:on={a.on} title={a.title} onclick={a.run}>{a.icon}</button>
    {/each}
    <button class="icon-btn" title="새로 고침" onclick={() => void refresh()}>⟳</button>
    {#if closable}<button class="icon-btn" title="패널 닫기" onclick={onclose}>✕</button>{/if}
  </div>

  {#if tabs.length > 0}
    <div class="tabs" role="tablist">
      {#each tabs as t (t.path)}
        {@const ch = t.rel ? changeOf.get(t.rel) : undefined}
        <div class="tab" class:on={active === t.path} class:live={activity?.path === t.path && busy.has(t.path)}>
          <button
            class="tab-name"
            role="tab"
            aria-selected={active === t.path}
            title={t.rel ?? t.path}
            onclick={() => {
              peek();
              activate(t.path);
            }}
          >
            {#if activity?.path === t.path && busy.has(t.path)}<span class="pen">✍</span>{/if}
            {t.name}
            {#if ch}<span class="badge st-{ch.status}">{BADGE[ch.status][0]}</span>{/if}
          </button>
          <button class="tab-x" title="탭 닫기" onclick={() => closeTab(t.path)}>×</button>
        </div>
      {/each}
    </div>
  {/if}

  {#if activity}
    <div class="activity" class:live={busy.has(activity.path)}>
      <span class="pen">{activity.kind === "replay" ? "⚙" : "✍"}</span>
      <span class="what">{relOf(activity.path) ?? shortPath(activity.path)}</span>
      <span class="phase">{activity.text}</span>
    </div>
  {/if}

  <div class="work" class:drawer-open={listOpen && !listBeside}>
    {#if listOpen}
      <div class="side" class:drawer={!listBeside}>
        {#if rootError}
          <div class="muted pad">작업 폴더를 열지 못했습니다: {rootError}</div>
        {:else if !root}
          <div class="muted pad">작업 폴더를 확인하는 중…</div>
        {:else}
          {#if root.git}
            <div class="sec">바뀐 파일 <span class="count">{changes.length}</span></div>
            {#if changes.length === 0}
              <div class="muted pad small">마지막 커밋 이후 바뀐 파일이 없습니다.</div>
            {/if}
            {#each changes as c (c.path)}
              {@const abs = absOf(c.path)}
              <button
                class="change"
                class:sel={active === abs}
                onclick={() => openFile(abs)}
                title={c.from ? `${c.from} → ${c.path}` : c.path}
              >
                <span class="badge st-{c.status}" title={BADGE[c.status][1]}>{BADGE[c.status][0]}</span>
                <span class="cpath"><span class="cdir">{dirOf(c.path)}</span><span class="cname">{baseOf(c.path)}</span></span>
                <span class="counts"
                  >{#if c.added}<span class="plus">+{c.added}</span>{/if}{#if c.removed}<span class="minus"
                      >-{c.removed}</span
                    >{/if}</span
                >
              </button>
            {/each}
          {/if}
          <div class="sec">폴더</div>
          <div class="tree">{@render treeLevel("", 0)}</div>
        {/if}
      </div>
    {/if}
    <div class="editor-wrap">
      <div class="editor" bind:this={host}></div>
      {#if !active}
        <div class="placeholder">
          Claude 가 파일을 고치면 그 파일이 여기 열리고, 바뀌는 내용이 입력되듯 보입니다. 위의 ☰ 목록을 누르면 바뀐
          파일과 폴더에서 골라 열어 볼 수도 있습니다.
        </div>
      {:else}
        {@const note = tabs.find((t) => t.path === active)?.note}
        {#if note}<div class="note">{note}</div>{/if}
      {/if}
    </div>
  </div>
</aside>

<style>
  .files {
    position: relative;
    flex: 0 0 auto;
    width: calc(var(--share, 0.5) * 100%);
    /* MIN_PANEL; the neighbour keeps --min-rest. */
    min-width: 320px;
    max-width: calc(100% - var(--min-rest, 360px));
    min-height: 0;
    display: flex;
    flex-direction: column;
    border-left: 1px solid var(--border);
    background: var(--surface);
    font-size: 0.86em;
  }
  .files.hidden {
    display: none;
  }
  .files.fill {
    position: absolute;
    inset: 0;
    width: auto;
    min-width: 0;
    max-width: none;
    border-left: none;
  }
  .slabel {
    flex: 0 1 auto;
    min-width: 0;
    max-width: 45%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 700;
    color: var(--text);
  }
  .files.overlay {
    position: absolute;
    left: 0;
    right: 0;
    top: var(--files-top, 0px);
    bottom: var(--chat-bottom-h, 0px);
    width: auto;
    min-width: 0;
    max-width: none;
    z-index: 6;
    border-left: none;
  }
  .grip {
    position: absolute;
    left: -3px;
    top: 0;
    bottom: 0;
    width: 7px;
    padding: 0;
    border: none;
    background: transparent;
    cursor: col-resize;
    z-index: 3;
  }
  .grip:hover {
    background: color-mix(in srgb, var(--accent) 45%, transparent);
  }
  .guide {
    position: fixed;
    width: 2px;
    margin-left: -1px;
    background: var(--accent);
    pointer-events: none;
    z-index: 2000;
  }
  .phead {
    flex: 0 0 auto;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 8px;
    border-bottom: 1px solid var(--border);
    background: var(--surface-2);
  }
  .root {
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }
  .branch {
    color: var(--muted);
    white-space: nowrap;
  }
  .spacer {
    flex: 1 1 auto;
  }
  .follow {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--text-2);
    white-space: nowrap;
    cursor: pointer;
  }
  button {
    font: inherit;
    color: var(--text);
    background: none;
    border: 1px solid transparent;
    border-radius: 4px;
    cursor: pointer;
  }
  .icon-btn,
  .list-toggle {
    padding: 0 6px;
    border-color: var(--border-2);
    white-space: nowrap;
  }
  .list-toggle.on,
  .icon-btn.on {
    border-color: var(--accent);
    color: var(--accent);
  }
  .icon-btn:hover,
  .list-toggle:hover {
    border-color: var(--accent);
  }
  .tabs {
    flex: 0 0 auto;
    display: flex;
    overflow-x: auto;
    border-bottom: 1px solid var(--border);
    background: var(--surface-2);
    scrollbar-width: thin;
  }
  .tab {
    flex: 0 0 auto;
    display: flex;
    align-items: center;
    border-right: 1px solid var(--border);
    color: var(--muted);
  }
  .tab.on {
    background: var(--bg);
    color: var(--text);
    box-shadow: inset 0 -2px 0 var(--accent);
  }
  .tab.live {
    animation: breathe 1.4s ease-in-out infinite;
  }
  .tab-name {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 4px 4px 4px 10px;
    color: inherit;
    border-radius: 0;
    font-family: ui-monospace, "JetBrains Mono", "D2Coding", monospace;
    white-space: nowrap;
  }
  .tab-x {
    padding: 0 8px 0 2px;
    color: var(--muted);
    border-radius: 0;
  }
  .tab-x:hover {
    color: var(--red);
  }
  .activity {
    flex: 0 0 auto;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 3px 10px;
    border-bottom: 1px solid var(--border);
    color: var(--text-2);
  }
  .activity.live {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    animation: breathe 1.4s ease-in-out infinite;
  }
  .activity .what {
    min-width: 0;
    font-family: ui-monospace, "JetBrains Mono", "D2Coding", monospace;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .activity .phase {
    margin-left: auto;
    color: var(--muted);
    white-space: nowrap;
  }
  @keyframes breathe {
    50% {
      background: color-mix(in srgb, var(--accent) 4%, transparent);
    }
  }
  .work {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    position: relative;
  }
  .side {
    flex: 0 0 230px;
    min-height: 0;
    overflow: auto;
    border-right: 1px solid var(--border);
    background: var(--surface);
    padding-bottom: 6px;
  }
  .side.drawer {
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    width: min(300px, 80%);
    z-index: 3;
    box-shadow: 4px 0 14px rgba(0, 0, 0, 0.35);
  }
  .editor-wrap {
    flex: 1 1 auto;
    min-width: 0;
    min-height: 0;
    position: relative;
    background: var(--bg);
  }
  .editor {
    position: absolute;
    inset: 0;
  }
  /* The hint stays readable beside an open drawer (a narrow panel, or a cell
     of the pop-out window, before any file is open). */
  .work.drawer-open .placeholder {
    padding-left: calc(min(300px, 80%) + 24px);
  }
  .placeholder {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    text-align: center;
    padding: 24px;
    color: var(--muted);
    background: var(--bg);
    line-height: 1.6;
  }
  .note {
    position: absolute;
    right: 10px;
    bottom: 8px;
    padding: 2px 8px;
    border-radius: 4px;
    background: var(--surface-2);
    color: var(--muted);
    font-size: 0.9em;
    pointer-events: none;
  }
  .sec {
    position: sticky;
    top: 0;
    z-index: 1;
    padding: 4px 10px;
    font-weight: 600;
    color: var(--text-2);
    background: var(--surface);
    border-bottom: 1px solid var(--border);
  }
  .count {
    color: var(--muted);
    font-weight: 400;
  }
  .change,
  .node {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    text-align: left;
    padding: 1px 10px;
    border-radius: 0;
    white-space: nowrap;
    min-width: 0;
  }
  .change:hover,
  .node:hover {
    background: var(--surface-2);
  }
  .change.sel,
  .node.sel {
    background: color-mix(in srgb, var(--accent) 22%, transparent);
  }
  .node.editing {
    animation: breathe 1.4s ease-in-out infinite;
  }
  .cpath {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    font-family: ui-monospace, "JetBrains Mono", "D2Coding", monospace;
  }
  .cdir {
    color: var(--muted);
  }
  .counts {
    font-family: ui-monospace, "JetBrains Mono", "D2Coding", monospace;
    font-size: 0.92em;
    display: inline-flex;
    gap: 4px;
  }
  .plus {
    color: var(--green);
  }
  .minus {
    color: var(--red);
  }
  .badge {
    flex: 0 0 auto;
    width: 1.3em;
    text-align: center;
    font-weight: 700;
    font-family: ui-monospace, "JetBrains Mono", "D2Coding", monospace;
    font-size: 0.9em;
  }
  .st-modified,
  .st-renamed,
  .st-copied {
    color: var(--yellow);
  }
  .st-added,
  .st-new {
    color: var(--green);
  }
  .st-deleted,
  .st-conflict {
    color: var(--red);
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }
  .node.file .name {
    flex: 1 1 auto;
  }
  .twisty {
    width: 1em;
    color: var(--muted);
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--yellow);
  }
  .pen {
    color: var(--accent);
  }
  .muted {
    color: var(--muted);
  }
  .err {
    color: var(--red);
  }
  .pad {
    padding: 6px 10px;
  }
  .small {
    font-size: 0.9em;
  }
</style>
