<script lang="ts">
  // Hosts one xterm.js instance bound to one engine pane.
  import { onMount, tick, untrack } from "svelte";
  import { Terminal } from "@xterm/xterm";
  import { FitAddon } from "@xterm/addon-fit";
  import { WebglAddon } from "@xterm/addon-webgl";
  import { Unicode11Addon } from "@xterm/addon-unicode11";
  import { WebLinksAddon } from "@xterm/addon-web-links";
  import { SearchAddon } from "@xterm/addon-search";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { readText, writeText } from "@tauri-apps/plugin-clipboard-manager";
  import "@xterm/xterm/css/xterm.css";
  import { writePane, resizePane, subscribePane, type PaneId } from "./ipc";
  import { handleKey } from "./keymap";
  import {
    adjustFontSize,
    settings,
    setLastInputPos,
    saveSettings,
    setShowComposer,
  } from "./settings.svelte";
  import { themeById } from "./themes";
  import {
    paneInfo,
    registerTermFocus,
    registerComposerFocus,
    broadcast,
    broadcastTargets,
  } from "./state.svelte";

  export interface MenuAction {
    label: string;
    run: () => void;
  }

  let {
    pane,
    focused = true,
    extraActions = [],
  }: { pane: PaneId; focused?: boolean; extraActions?: MenuAction[] } = $props();

  let host: HTMLDivElement;
  let menu = $state<{ x: number; y: number } | null>(null);
  let term = $state<Terminal>()!;
  let refit: (() => void) | undefined;

  // ── Last-command chip ─────────────────────────────────────────────────
  // Pin the user's most recently *submitted command* (up to 4 lines) inside
  // the pane, in a draggable chip. We reconstruct the line from the keystrokes
  // the user types (term.onData); it's accurate for typed/pasted prompts. We
  // mirror Claude's submit rule: Enter submits UNLESS the line ends with "\"
  // or is a modified Enter (Shift/Alt) — those insert a newline — so multi-line
  // prompts land as one command. Trivial confirmations (y/n, menu numbers,
  // single keys) are ignored so a one-key answer to a tool prompt doesn't
  // replace the real command. Known
  // limits: a ↑-recalled line arrives on the OUTPUT stream (not here), so it
  // shows the previous captured value, and heavy in-line editing (arrows)
  // reconstructs only approximately.
  let lastInput = $state("");
  let inputBuf = ""; // in-progress line; committed to lastInput on plain Enter
  let inPaste = false; // inside a bracketed-paste block (\x1b[200~ … \x1b[201~)
  // Drag state for repositioning the chip (position persists in settings).
  let bannerEl = $state<HTMLDivElement>();
  let dragging = $state(false);
  let dragOrigin = { px: 0, py: 0, x: 0, y: 0 };

  function commitLine() {
    const t = inputBuf.trim();
    inputBuf = "";
    // Ignore trivial confirmations so the chip keeps the last *real* command.
    if (!t || t.length <= 1 || /^(y|n|yes|no|\d{1,3})$/i.test(t)) return;
    lastInput = t;
  }

  function bannerPointerDown(e: PointerEvent) {
    dragging = true;
    dragOrigin = {
      px: e.clientX,
      py: e.clientY,
      x: settings.lastInputPos?.x ?? 8,
      y: settings.lastInputPos?.y ?? 6,
    };
    bannerEl?.setPointerCapture(e.pointerId);
    e.preventDefault();
    e.stopPropagation();
  }
  function bannerPointerMove(e: PointerEvent) {
    if (!dragging) return;
    const bw = bannerEl?.offsetWidth ?? 0;
    const bh = bannerEl?.offsetHeight ?? 0;
    const maxX = Math.max(0, host.clientWidth - bw);
    const maxY = Math.max(0, host.clientHeight - bh);
    const x = Math.max(0, Math.min(maxX, dragOrigin.x + (e.clientX - dragOrigin.px)));
    const y = Math.max(0, Math.min(maxY, dragOrigin.y + (e.clientY - dragOrigin.py)));
    setLastInputPos(x, y);
  }
  function bannerPointerUp(e: PointerEvent) {
    if (!dragging) return;
    dragging = false;
    bannerEl?.releasePointerCapture(e.pointerId);
    saveSettings(); // persist the resting position once, on drop
  }

  function trackInput(data: string) {
    let i = 0;
    while (i < data.length) {
      // Bracketed paste: take the pasted text literally, drop the markers.
      if (inPaste) {
        const end = data.indexOf("\x1b[201~", i);
        if (end === -1) {
          inputBuf += data.slice(i);
          return;
        }
        inputBuf += data.slice(i, end);
        inPaste = false;
        i = end + 6;
        continue;
      }
      if (data.startsWith("\x1b[200~", i)) {
        inPaste = true;
        i += 6;
        continue;
      }
      // Shift+Enter (both encodings this pane emits) → newline within the line.
      if (data.startsWith("\x1b\r", i)) {
        inputBuf += "\n";
        i += 2;
        continue;
      }
      // Modified Enter in kitty mode (\x1b[13;<mods>u, ANY modifier) → newline.
      if (data.startsWith("\x1b[13;", i)) {
        const u = data.indexOf("u", i + 5);
        if (u !== -1 && /^\d+$/.test(data.slice(i + 5, u))) {
          inputBuf += "\n";
          i = u + 1;
          continue;
        }
      }
      const ch = data[i];
      if (ch === "\r" || ch === "\n") {
        // Enter submits ONLY when the line doesn't end with a backslash.
        // "\"+Enter is a line-continuation (newline) — the same rule Claude and
        // the shell use — so drop the backslash and keep composing. (Shift/Alt
        // +Enter, handled above, are newlines too.)
        if (inputBuf.endsWith("\\")) {
          inputBuf = inputBuf.slice(0, -1) + "\n";
        } else {
          commitLine();
        }
        i += 1;
        continue;
      }
      if (ch === "\x7f" || ch === "\b") {
        inputBuf = inputBuf.slice(0, -1); // backspace
        i += 1;
        continue;
      }
      if (ch === "\x15" || ch === "\x03") {
        inputBuf = ""; // Ctrl+U (kill line) / Ctrl+C (abandon)
        i += 1;
        continue;
      }
      if (ch === "\x1b") {
        // Unhandled escape (arrows, Home/End, …): skip the whole sequence so
        // cursor moves don't land as literal text.
        i += 1;
        if (data[i] === "[" || data[i] === "O") {
          i += 1;
          while (i < data.length && !(data[i] >= "@" && data[i] <= "~")) i += 1;
          i += 1; // consume the final byte
        } else {
          i += 1; // ESC + single char (Alt+key, lone Esc)
        }
        continue;
      }
      if (ch < " ") {
        i += 1; // other control bytes: ignore
        continue;
      }
      inputBuf += ch;
      i += 1;
    }
  }

  // ── 하단 고정 입력창 (Composer) ────────────────────────────────────────
  // 문제: 긴 출력을 위로 스크롤해 읽는 도중 다음 프롬프트를 타이핑하면 화면이
  // 매 키 입력마다 맨 아래로 튄다. xterm이 "사용자 입력 = 최신 출력을 봐야
  // 한다"고 보고 강제로 스크롤하기 때문(scrollOnUserInput). 반대로 *출력*은
  // 스크롤을 건드리지 않는다 — 읽던 자리는 그대로 유지된다.
  // 해결: pane 맨 아래에 터미널과 분리된 입력칸을 붙인다. 여기 타이핑하는
  // 동안 xterm은 아무 입력도 받지 않으므로 스크롤 위치가 그대로 있고, Enter를
  // 누르는 순간에만 텍스트가 PTY로 한 번에 들어간다.
  // 자리 차지 방식: 입력창은 세로 스택 안에 그대로 놓인다. 내용이 길어져 높이가
  // 자라면 그만큼 터미널 칸이 실제로 줄어들고(가려지지 않는다), 줄어든 칸을
  // ResizeObserver가 감지해 xterm과 PTY 크기까지 맞춘다. 그 대가로 줄 수가 바뀌는
  // 순간마다 안에서 돌던 TUI가 화면을 한 번 다시 그린다 — 키 하나마다가 아니라
  // 줄이 늘고 줄 때만이라 감당할 만하다고 보고 고른 쪽이다.
  const composerOn = $derived(settings.showComposer ?? true);

  // ── IME 조합 가드 ────────────────────────────────────────────────────────
  // 한글·일본어는 자모/가나를 모아 한 글자를 만드는 "조합(composition)" 단계를
  // 거치고, 그 동안 글자는 아직 어느 입력 요소에도 확정되지 않은 채 IME 안에만
  // 있다. xterm은 조합이 끝나면 자기 숨은 textarea에서
  // `value.substring(조합 시작 위치)` 를 잘라 PTY로 보내는데(CompositionHelper),
  // 그 textarea는 **blur될 때만** 비워진다. 그래서 조합 도중에 키보드 포커스가
  // 움직이면 시작 위치와 실제 값이 어긋나, 이미 친 글이 통째로 한 번 더
  // 들어가거나 반대로 사라진다 — 사용자가 겪은 "타이핑 중 갑자기 작성한 게
  // 다시 붙여넣어지는" 증상의 정체다.
  //
  // amux는 사람이 아무것도 안 해도 포커스를 옮기는 자리가 여럿이라(엔진이
  // pane을 waiting으로 바꿀 때, 활성 pane이 바뀔 때, focusTerm 호출 등) 이
  // 함정을 특히 자주 밟는다. 그래서 조합이 진행 중인 동안에는 **자동 포커스
  // 이동을 전부 보류**한다. 조합이 끝나면 이 값이 false로 돌아가고, 이걸 읽는
  // $effect들이 다시 돌면서 미뤄 둔 포커스 이동을 그때 수행한다.
  let imeComposing = $state(false);

  // ── 오타성 스크롤 가드 ───────────────────────────────────────────────────
  // xterm은 스크롤백이 없는 화면(vim·tmux 같은 전체화면 앱이 쓰는 "대체 화면
  // 버퍼")에서 **휠 이벤트를 위/아래 방향키로 바꿔 앱에 보낸다**. 원래는 마우스
  // 지원이 없는 앱에서도 휠로 스크롤할 수 있게 하려는 배려다.
  //
  // 그런데 노트북 터치패드에서는 타이핑 중 손바닥이나 손가락이 살짝 스치기만
  // 해도 이 변환이 일어난다. 그리고 Claude Code는 ↑ 를 받으면 **직전에 보낸
  // 프롬프트를 입력창에 통째로 되돌려 놓는다.** 즉 사용자가 아무 키도 안 눌렀는데
  // 이미 쓴 글이 다시 붙여넣어진 것처럼 보인다.
  //
  // 그래서 마지막 키 입력 이후 이 시간 안에 들어온 휠은 "타이핑 중 스친 것"으로
  // 보고 방향키 변환을 막는다. 손을 멈추고 의도적으로 스크롤하는 경우는 이 창을
  // 벗어나므로 정상 동작한다.
  const TYPING_SCROLL_GUARD_MS = 800;
  let lastKeyAt = 0;

  let draft = $state("");
  let ta = $state<HTMLTextAreaElement>();
  let composerFocused = $state(false);

  // Grow the box with its content, up to ~45% of the pane; then scroll inside.
  function autosize() {
    if (!ta) return;
    // 최대 높이의 기준은 **pane 전체 높이**(host의 부모인 .term-stack)여야 한다.
    // 터미널 칸(host)을 기준으로 삼으면 진동한다: 입력창이 커지면 터미널이 줄고
    // → 기준이 줄어 최대치가 낮아지고 → 입력창이 다시 줄고 → 터미널이 늘고 …
    // .term-stack은 pane에 고정(position: absolute; inset: 0)이라 흔들리지 않는다.
    const paneH = host?.parentElement?.clientHeight ?? host?.clientHeight ?? 400;
    const max = Math.max(80, Math.round(paneH * 0.45));
    ta.style.height = "auto";
    // +2: box-sizing is border-box app-wide, but scrollHeight excludes borders.
    ta.style.height = Math.min(ta.scrollHeight + 2, max) + "px";
  }

  /** Send the draft to this pane's PTY. `submit` also presses Enter. */
  async function sendDraft(submit: boolean) {
    const text = draft.replace(/\r\n?/g, "\n").replace(/\s+$/, "");
    if (!text) return;

    // 여러 줄은 bracketed paste로 감싼다: 안 그러면 줄바꿈마다 Enter로 읽혀
    // 한 줄씩 제출돼 버린다. 한 줄이면 그냥 타이핑한 것과 똑같은 바이트를 보낸다.
    const body = text.replace(/\n/g, "\r");
    const multi = text.includes("\n");
    const bracketed = multi && term?.modes.bracketedPasteMode === true;
    const payload = multi ? (bracketed ? `\x1b[200~${body}\x1b[201~` : body) : body;
    // Broadcast mode mirrors the composer just like it mirrors typing.
    const targets = broadcast.on && focused ? broadcastTargets(pane) : [];

    // 첫 write가 실패하면(pane 종료 등) 작성 중이던 글이 사라지지 않도록
    // draft는 전송이 확인된 뒤에만 비운다.
    const head = !multi && submit ? body + "\r" : payload;
    try {
      await writePane(pane, head);
    } catch {
      return;
    }
    for (const target of targets) void writePane(target, head);
    if (multi && submit) {
      // 붙여넣기 블록을 앱(Claude Code 등)이 먼저 소화하도록 Enter는 한 박자 뒤.
      setTimeout(() => {
        void writePane(pane, "\r");
        for (const target of targets) void writePane(target, "\r");
      }, 40);
    }

    draft = "";
    await tick();
    autosize();
    if (submit) {
      inputBuf = text; // feed the last-command chip, same as typed input
      commitLine();
    }
    // 보낸 뒤에는 결과를 봐야 하니 맨 아래로. (튀는 게 싫었던 건 "타이핑 중"이지
    // "보낸 뒤"가 아니다.)
    term?.scrollToBottom();
    ta?.focus();
  }

  // 입력칸이 비어 있을 때, "한 글자씩 반응하는 UI"로 가야 하는 키는 터미널로
  // 넘긴다. `/`를 누르면 Claude Code가 슬래시 명령 자동완성을 띄우고, 옵션
  // 메뉴는 ↑↓로 고르고, Tab은 모드를 바꾼다 — 이런 건 입력칸에서 문장을
  // 조립해 한 번에 보내는 방식으로는 쓸 수가 없다. 그래서 그 순간 포커스를
  // 터미널로 옮기고 누른 키를 그대로 흘려보낸다(= 원래 터미널에 친 것과 동일).
  const HANDOFF_PREFIX = ["/", "!", "#", "@"];

  function handOff(data: string) {
    void writePane(pane, data);
    if (broadcast.on && focused) {
      for (const target of broadcastTargets(pane)) void writePane(target, data);
    }
    // 포커스 이동은 이 키 이벤트가 끝난 뒤로 미룬다. keydown 도중에 xterm의
    // 숨은 textarea로 포커스를 옮기면 이어지는 keypress를 xterm이 받아 같은
    // 글자를 PTY에 한 번 더 보낼 수 있다("//"). preventDefault로도 대개 막히지만,
    // 위 Shift+Enter 주석의 그 함정과 같은 계열이라 순서로 아예 차단한다.
    setTimeout(() => term?.focus(), 0);
  }

  function composerKey(e: KeyboardEvent) {
    // 입력창에서 치는 동안에도 "타이핑 중"이다 — 위쪽 터미널 위로 손이 스쳐
    // 휠이 들어오면 막아야 하므로 여기서도 시각을 찍는다.
    lastKeyAt = Date.now();
    // 한글/일본어 IME 조합 중의 Enter는 "글자 확정"이지 전송이 아니다.
    // 이 가드가 없으면 "안녕"을 확정하는 Enter가 그대로 전송돼 버린다.
    if (e.isComposing || e.keyCode === 229) return;

    if (draft === "" && !e.ctrlKey && !e.altKey && !e.metaKey) {
      if (e.key.length === 1 && HANDOFF_PREFIX.includes(e.key)) {
        e.preventDefault();
        handOff(e.key);
        return;
      }
      if (!e.shiftKey && (e.key === "ArrowUp" || e.key === "ArrowDown")) {
        e.preventDefault();
        handOff(e.key === "ArrowUp" ? "\x1b[A" : "\x1b[B");
        return;
      }
      if (e.key === "Tab") {
        e.preventDefault();
        handOff(e.shiftKey ? "\x1b[Z" : "\t");
        return;
      }
    }

    if (e.key === "Enter" && !e.shiftKey && !e.altKey) {
      e.preventDefault();
      void sendDraft(!e.ctrlKey); // Ctrl+Enter: 넣기만 하고 제출은 안 함
      return;
    }
    if (e.key === "Escape") {
      e.preventDefault();
      term?.focus();
    }
  }

  // 엔진의 `waiting` = 턴 진행 중 Claude가 권한/선택을 묻는 순간 (턴이 끝난 뒤의
  // 유휴 "입력 기다림" 알림은 엔진이 걸러낸다). 즉 키보드가 입력칸이 아니라
  // 터미널에 있어야 하는 바로 그 시점.
  const waitingInput = $derived(paneInfo(pane)?.status === "waiting");
  let prevWaiting = false;
  $effect(() => {
    const w = waitingInput;
    // 전환(false→true) 시에만, 그리고 쓰던 글이 없을 때만 넘긴다. 작성 중이면
    // 절대 뺏지 않는다 — 반쯤 쓴 프롬프트가 TUI로 새어 들어가면 안 되니까.
    //
    // `draft === ""` 만으로는 부족하다: 한글을 치는 중에는 글자가 아직 IME 안에
    // 머물러 draft가 여전히 빈 문자열이다. 그 순간 포커스를 뺏으면 조합이
    // 끊기면서 이미 친 글이 통째로 다시 들어간다. 게다가 이 effect는 사람이
    // 아니라 **엔진의 상태 변화**가 방아쇠라, 사용자 입장에서는 아무 조작도 안
    // 했는데 갑자기 벌어지는 일이 된다. 그래서 조합 중에는 아예 건너뛴다.
    if (w && !prevWaiting && !imeComposing && untrack(() => composerFocused && draft === "")) {
      term?.focus();
    }
    prevWaiting = w;
  });

  $effect(() => {
    void settings.fontSize;
    void composerOn;
    void tick().then(autosize);
  });

  // ── Activity widgets ──────────────────────────────────────────────────
  // Two visualizers of the SAME signal — this pane's output byte-rate (never
  // its content): an oscilloscope waveform paired with an Arc Reactor core,
  // top-right. The bound canvases + accent color live here; the byte counter
  // and rAF loop live in onMount, next to the chunk stream.
  let waveCanvas: HTMLCanvasElement;
  let arcCanvas: HTMLCanvasElement;
  let waveColor = "#7aa2f7"; // accent → wave + arc strokes
  let waveRGB = "122, 162, 247"; // accent as "r, g, b" for rgba() fills
  function readAccent() {
    let h = getComputedStyle(document.documentElement)
      .getPropertyValue("--accent")
      .trim()
      .replace("#", "");
    if (h.length === 3) h = h.split("").map((c) => c + c).join("");
    if (h.length < 6) return;
    const r = parseInt(h.slice(0, 2), 16);
    const g = parseInt(h.slice(2, 4), 16);
    const b = parseInt(h.slice(4, 6), 16);
    if (Number.isNaN(r) || Number.isNaN(g) || Number.isNaN(b)) return;
    waveColor = "#" + h.slice(0, 6);
    waveRGB = `${r}, ${g}, ${b}`;
  }

  let search: SearchAddon;
  let searchOpen = $state(false);
  let searchQuery = $state("");

  const searchOptions = { decorations: { matchOverviewRuler: "#7aa2f7", activeMatchColorOverviewRuler: "#ff9e64" } };

  function closeSearch() {
    searchOpen = false;
    searchQuery = "";
    search?.clearDecorations();
    term?.clearSelection();
    term?.focus();
  }

  // kitty keyboard protocol state of the app in this pane (e.g. Claude Code).
  // When active, the app wants shift+arrows itself (input highlighting) and
  // the CSI-u encodings for Esc / modified Enter.
  const kitty = $derived(paneInfo(pane)?.meta.kitty_keyboard === true);

  // Terminal-level keyboard selection: shift+arrow extends a visual
  // selection anchored at the cursor. `chars` counts horizontal presses
  // (negative = leftward) so cut can translate to Backspace/Delete;
  // vertical selections can only be copied, not cut.
  let kbSel: { anchor: number; extent: number; chars: number; vertical: boolean } | null = null;

  function extendKeyboardSelection(key: string) {
    const buf = term.buffer.active;
    const cols = term.cols;
    if (!kbSel) {
      const cursor = (buf.baseY + buf.cursorY) * cols + buf.cursorX;
      kbSel = { anchor: cursor, extent: cursor, chars: 0, vertical: false };
    }
    if (key === "ArrowUp" || key === "ArrowDown") {
      kbSel.vertical = true;
      kbSel.extent += key === "ArrowUp" ? -cols : cols;
    } else {
      const dir = key === "ArrowLeft" ? -1 : 1;
      kbSel.chars += dir;
      kbSel.extent += dir;
      // Step over wide-char spacer cells so CJK glyphs stay whole.
      const line = buf.getLine(Math.floor(kbSel.extent / cols));
      if (line?.getCell(kbSel.extent % cols)?.getWidth() === 0) {
        kbSel.extent += dir;
      }
    }
    const max = buf.length * cols - 1;
    kbSel.extent = Math.min(max, Math.max(0, kbSel.extent));
    const start = Math.min(kbSel.anchor, kbSel.extent);
    const end = Math.max(kbSel.anchor, kbSel.extent);
    term.select(start % cols, Math.floor(start / cols), Math.max(1, end - start));
  }

  function clearKeyboardSelection() {
    if (kbSel) {
      kbSel = null;
      term.clearSelection();
    }
  }

  /// Cut: copy, then — for horizontal keyboard selections — actually delete
  /// the characters from the app's input (Backspace for leftward selections,
  /// Delete for rightward). Mouse/vertical selections copy only.
  async function cutSelection() {
    if (!term.hasSelection()) return;
    await copySelection();
    if (kbSel && !kbSel.vertical && kbSel.chars !== 0) {
      const n = Math.abs(kbSel.chars);
      const seq = kbSel.chars < 0 ? "\x7f".repeat(n) : "\x1b[3~".repeat(n);
      await writePane(pane, seq);
    }
    kbSel = null;
    term.clearSelection();
  }

  onMount(() => {
    term = new Terminal({
      allowProposedApi: true,
      scrollback: 10_000,
      fontFamily: "monospace",
      fontSize: settings.fontSize,
      theme: themeById(settings.theme).term,
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.loadAddon(new Unicode11Addon());
    term.unicode.activeVersion = "11";
    search = new SearchAddon();
    term.loadAddon(search);
    term.loadAddon(
      new WebLinksAddon((e, uri) => {
        if (e.ctrlKey) void openUrl(uri);
      }),
    );
    // App shortcuts (split/navigate/...) win over the terminal; everything
    // else (Ctrl+C, Tab, F-keys...) flows through to the shell untouched.
    term.attachCustomKeyEventHandler((e) => {
      // IME 조합 중의 키는 우리가 해석하지 않고 xterm의 조합 처리에 그대로
      // 맡긴다. 조합 중에는 keyCode가 229로 뭉뚱그려 오고 `e.key`도 실제 키와
      // 다르게 실릴 수 있어서, 여기서 단축키로 가로채면 조합이 중간에 끊기고
      // 그 시점의 조합 버퍼가 통째로 다시 흘러나온다.
      if (e.isComposing || e.keyCode === 229) return true;
      // Shift+Enter → 줄바꿈: kitty 모드 앱(Claude Code)에는 CSI-u 인코딩,
      // 그 외에는 ESC+CR (iTerm2 /terminal-setup과 동일한 매핑).
      if (
        e.type === "keydown" &&
        e.key === "Enter" &&
        e.shiftKey &&
        !e.ctrlKey &&
        !e.altKey
      ) {
        // preventDefault가 없으면 브라우저가 이어서 keypress를 쏘고, xterm은
        // 거기서 charCode 13을 그대로 PTY에 보낸다(`_keyPress` → `\r`). 우리가
        // 보낸 줄바꿈 직후 Enter가 한 번 더 들어가 프롬프트가 제출돼 버리므로
        // (핸들러가 false를 반환해도 xterm은 preventDefault를 대신 해주지 않음)
        // keypress 자체를 막아야 한다.
        e.preventDefault();
        void writePane(pane, kitty ? "\x1b[13;2u" : "\x1b\r");
        return false;
      }
      // kitty 모드에서 plain Esc는 CSI 27u로 보고해야 함 (프로토콜 규약).
      if (
        e.type === "keydown" &&
        e.key === "Escape" &&
        kitty &&
        !e.ctrlKey &&
        !e.altKey &&
        !e.shiftKey
      ) {
        void writePane(pane, "\x1b[27u");
        return false;
      }
      // Shift+방향키 → 터미널 키보드 선택 (커서 기준 하이라이트 확장,
      // copy-on-select로 자동 복사). Claude Code는 입력창 키보드 선택이
      // 아직 없으므로(anthropics/claude-code#23396) 항상 터미널이 갖는다.
      if (
        e.type === "keydown" &&
        e.shiftKey &&
        !e.ctrlKey &&
        !e.altKey &&
        e.key.startsWith("Arrow")
      ) {
        extendKeyboardSelection(e.key);
        return false;
      }
      if (e.type === "keydown" && e.ctrlKey && e.shiftKey && !e.altKey) {
        // Terminal-convention clipboard keys, via the Rust clipboard
        // (navigator.clipboard is unreliable in WebKitGTK).
        if (e.code === "KeyC" && term.hasSelection()) {
          void copySelection();
          return false;
        }
        if (e.code === "KeyV") {
          // 브라우저 네이티브 paste가 한 번 더 붙는 것 방지
          e.preventDefault();
          void pasteClipboard();
          return false;
        }
        // 스크롤백 검색
        if (e.code === "KeyF") {
          searchOpen = true;
          return false;
        }
      }
      if (e.type === "keydown" && e.ctrlKey && !e.shiftKey && !e.altKey) {
        // Ctrl+C copies when text is selected; otherwise it stays SIGINT.
        if (e.code === "KeyC" && term.hasSelection()) {
          void copySelection().then(() => {
            kbSel = null;
            term.clearSelection();
          });
          return false;
        }
        // Ctrl+X 잘라내기: 복사 후, 키보드 선택(가로)은 입력에서 실제 삭제.
        // 선택이 없으면 nano/emacs 등이 쓰는 원래 Ctrl+X로 동작.
        if (e.code === "KeyX" && term.hasSelection()) {
          void cutSelection();
          return false;
        }
        // Ctrl+V always pastes (readline's literal-next is Ctrl+Shift+V
        // territory for the rare user who needs it... which we also use
        // for paste, so literal-next is effectively retired here).
        if (e.code === "KeyV") {
          // 브라우저 네이티브 paste가 한 번 더 붙는 것 방지
          e.preventDefault();
          void pasteClipboard();
          return false;
        }
      }
      return !handleKey(e);
    });
    // 휠 → 방향키 변환을 가로챈다. `false` 를 돌려주면 xterm 은 그 휠 이벤트를
    // 아예 처리하지 않으므로, 앱으로 ESC[A / ESC[B 가 나가지 않는다.
    term.attachCustomWheelEventHandler((e) => {
      // Ctrl+휠은 글꼴 확대/축소 — 터미널 호스트의 onwheel 이 처리하므로 넘긴다.
      if (e.ctrlKey) return false;
      // 방금까지 타이핑하고 있었다면 손이 스친 것으로 보고 삼킨다.
      if (Date.now() - lastKeyAt < TYPING_SCROLL_GUARD_MS) return false;
      return true;
    });
    // Linux terminal convention: selecting text copies it.
    let selectionTimer: ReturnType<typeof setTimeout> | undefined;
    term.onSelectionChange(() => {
      clearTimeout(selectionTimer);
      selectionTimer = setTimeout(() => {
        if (term.hasSelection()) void writeText(term.getSelection());
      }, 150);
    });
    term.open(host);
    // 마지막 키 입력 시각은 **DOM에서 직접** 찍는다. 아래 휠 가드가 "지금
    // 타이핑 중인가"를 판단하는 근거다.
    //
    // 왜 xterm의 키 핸들러가 아니라 여기인가: 그쪽에는 IME 조합 중 키를 xterm에
    // 그대로 넘기는 조기 반환이 맨 앞에 있어서, **한글을 치는 동안에는 시각이
    // 한 번도 갱신되지 않았다.** 한글 조합 중 키는 keyCode가 229로 오기 때문이다.
    // 그 결과 "입력창에 칠 때는 멀쩡한데 터미널에 칠 때만 증상이 남는" 비대칭이
    // 생겼다(입력창 쪽 composerKey는 조기 반환보다 앞에서 시각을 찍고 있었다).
    // 캡처 단계의 DOM 리스너는 xterm 내부 사정과 무관하게 항상 먼저 실행된다.
    const stampKey = () => (lastKeyAt = Date.now());
    host.addEventListener("keydown", stampKey, true);
    // xterm은 조합 진행 여부를 공개 API로 내주지 않으므로, `term.open()` 이
    // 만들어 둔 숨은 textarea에서 조합 시작/끝 이벤트를 직접 관찰한다.
    const imeOn = () => (imeComposing = true);
    const imeOff = () => (imeComposing = false);
    const xtermTextarea = term.textarea;
    xtermTextarea?.addEventListener("compositionstart", imeOn);
    xtermTextarea?.addEventListener("compositionend", imeOff);

    // ── xterm 한글 조합 버그 우회 ──────────────────────────────────────────
    // xterm의 숨은 textarea는 **포커스가 빠질 때만** 비워진다. 그래서 한 번
    // 포커스를 잡은 뒤로 친 글자가 거기 계속 쌓인다. 문제는 조합 처리 코드가
    // 그 누적된 값을 통째로 보내 버리는 분기를 갖고 있다는 점이다.
    //
    //   // CompositionHelper._handleAnyTextareaChanges()
    //   const diff = newValue.replace(oldValue, '');        // 접두사 제거가 아니라 "문자열 찾아 바꾸기"
    //   if (newValue.length > oldValue.length)      triggerDataEvent(diff);
    //   else if (newValue.length < oldValue.length) triggerDataEvent(DEL);
    //   else if (newValue !== oldValue)             triggerDataEvent(newValue);  // ← 누적분 전체를 보낸다
    //
    // 한글은 한 글자를 **제자리에서 바꿔 가며** 완성한다("하"→"한"). 길이는 그대로인데
    // 내용만 바뀌므로 위 세 번째 분기에 정확히 걸린다. 그 순간 textarea에 쌓여 있던
    // "지금까지 친 것 전부"가 한 덩어리로 PTY에 다시 들어간다 — 사용자가 겪은
    // "작성한 게 자동으로 다시 붙여넣어지는" 증상이다. 두 번째 분기도 위험하다:
    // `replace`는 접두사 제거가 아니라서 옛 값이 새 값 안에 없으면(제자리 수정이면
    // 대개 없다) diff가 새 값 전체가 된다.
    //
    // 영문에는 제자리 수정이 없어서 안 터지고, 하단 입력창은 xterm을 아예 거치지
    // 않아서(완성된 문장을 한 번에 PTY로 보냄) 멀쩡했다 — 관찰된 비대칭 그대로다.
    //
    // 우회: **조합이 시작되는 순간 textarea를 비운다.** 캡처 단계로 달았기 때문에
    // xterm 자신의 compositionstart 처리보다 먼저 돌고, xterm은 비워진 값을 기준으로
    // 조합 시작 위치를 0으로 잡는다. 그러면 textarea에는 항상 "지금 조합 중인 글자"
    // 하나뿐이라, 위 분기가 터지더라도 흘러나올 수 있는 최대치가 그 한 글자로 묶인다.
    const resetImeBuffer = () => {
      if (xtermTextarea) xtermTextarea.value = "";
    };
    host.addEventListener("compositionstart", resetImeBuffer, true);
    try {
      const webgl = new WebglAddon();
      webgl.onContextLoss(() => webgl.dispose()); // falls back to DOM renderer
      term.loadAddon(webgl);
    } catch {
      // WebKitGTK without a usable WebGL context: DOM renderer is fine.
    }

    // ── Thinking Waveform engine ──────────────────────────────────────
    // Each frame folds the bytes seen since the last frame into one smoothed
    // sample (fast attack, slow release) and pushes it into a fixed ~3s
    // history stretched across the strip. That 3s window doubles as a linger:
    // a TUI's spinner redrawing ~1–2×/s keeps the loop awake, so it reads as a
    // continuous ripple, not a wake/sleep strobe. Once the window is genuinely
    // flat the loop sleeps and clears — idle panes draw nothing, cost nothing.
    const HISTORY = 180; // samples ≈ 3s @ 60fps — the visible time window
    const RELEASE = 0.84; // per-frame decay of the smoothed level
    const SCALE = 600; // bytes/frame → ~full height (sqrt-compressed)
    const history: number[] = []; // oldest→newest; newest drawn at the right edge
    let bytesSinceFrame = 0;
    let smoothed = 0;
    let waveRaf = 0;
    let waveRunning = false;
    let arcAngle = 0; // Arc Reactor sweep rotation (persists across frames)

    const levelFromBytes = (bytes: number) =>
      bytes <= 0 ? 0 : Math.min(1, Math.sqrt(bytes / SCALE));
    // Gamma < 1 lifts light activity (a spinner's trickle) so it still reads
    // boldly, while heavy streaming still tops out near full.
    const GAIN = 0.7;
    const shape = (v: number) => Math.pow(v, GAIN);
    const recentPeak = () => {
      let p = 0;
      for (let i = 0; i < history.length; i++) if (history[i] > p) p = history[i];
      return p;
    };
    // Size each canvas to its CSS box (dpr-aware) and clear it. Returns null
    // when it's hidden/zero-sized so the caller bails.
    function prep(c: HTMLCanvasElement) {
      const w = c.clientWidth,
        h = c.clientHeight;
      if (w <= 0 || h <= 0) return null;
      const dpr = window.devicePixelRatio || 1;
      const nw = Math.round(w * dpr),
        nh = Math.round(h * dpr);
      if (c.width !== nw || c.height !== nh) {
        c.width = nw;
        c.height = nh;
      }
      const ctx = c.getContext("2d");
      if (!ctx) return null;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, w, h);
      return { ctx, w, h };
    }

    // Waveform — a mirrored oscilloscope of the last ~3s of activity.
    function drawWave() {
      const p = prep(waveCanvas);
      if (!p) return;
      const { ctx, w, h } = p;
      const n = history.length;
      if (n < 2) return;
      const mid = h / 2;
      const amp = mid - 2;
      const step = w / (HISTORY - 1);
      const x0 = w - (n - 1) * step; // right-align the newest sample
      const yTop = (v: number) => mid - shape(v) * amp;
      const yBot = (v: number) => mid + shape(v) * amp;
      const peak = shape(recentPeak());

      ctx.beginPath();
      ctx.moveTo(x0, yTop(history[0]));
      for (let i = 1; i < n; i++) ctx.lineTo(x0 + i * step, yTop(history[i]));
      for (let i = n - 1; i >= 0; i--) ctx.lineTo(x0 + i * step, yBot(history[i]));
      ctx.closePath();
      const grad = ctx.createLinearGradient(0, 0, 0, h);
      grad.addColorStop(0, `rgba(${waveRGB}, 0)`);
      grad.addColorStop(0.5, `rgba(${waveRGB}, ${0.55 * peak})`);
      grad.addColorStop(1, `rgba(${waveRGB}, 0)`);
      ctx.fillStyle = grad;
      ctx.fill();

      ctx.beginPath();
      ctx.moveTo(x0, yTop(history[0]));
      for (let i = 1; i < n; i++) ctx.lineTo(x0 + i * step, yTop(history[i]));
      ctx.strokeStyle = waveColor;
      ctx.lineWidth = 1.5;
      ctx.globalAlpha = 0.25 + 0.7 * peak;
      ctx.shadowColor = waveColor;
      ctx.shadowBlur = 8;
      ctx.stroke();
      ctx.globalAlpha = 1;
      ctx.shadowBlur = 0;
    }

    // Arc Reactor — a ring with a rotating sweep + core, Iron-Man HUD style.
    function drawArc() {
      const p = prep(arcCanvas);
      if (!p) return;
      const { ctx, w, h } = p;
      const lvl = shape(smoothed);
      const pk = shape(recentPeak());
      const cx = w / 2,
        cy = h / 2;
      const R = Math.min(w, h) / 2 - 4;
      arcAngle = (arcAngle + 0.04 + lvl * 0.5) % (Math.PI * 2); // spin ∝ throughput
      ctx.lineWidth = 2 + pk * 2.5; // base ring
      ctx.strokeStyle = `rgba(${waveRGB}, ${0.18 + 0.32 * pk})`;
      ctx.beginPath();
      ctx.arc(cx, cy, R, 0, Math.PI * 2);
      ctx.stroke();
      ctx.strokeStyle = waveColor; // bright rotating sweep (~90°)
      ctx.globalAlpha = 0.5 + 0.5 * pk;
      ctx.shadowColor = waveColor;
      ctx.shadowBlur = 4 + 8 * pk;
      ctx.beginPath();
      ctx.arc(cx, cy, R, arcAngle, arcAngle + Math.PI * 0.5);
      ctx.stroke();
      ctx.shadowBlur = 0;
      ctx.globalAlpha = 1;
      const cr = Math.max(0.2, 1.5 + lvl * (R - 3)); // core glow
      const rg = ctx.createRadialGradient(cx, cy, 0, cx, cy, cr);
      rg.addColorStop(0, `rgba(${waveRGB}, ${0.6 + 0.4 * lvl})`);
      rg.addColorStop(1, `rgba(${waveRGB}, 0)`);
      ctx.fillStyle = rg;
      ctx.beginPath();
      ctx.arc(cx, cy, cr, 0, Math.PI * 2);
      ctx.fill();
    }

    function waveTick() {
      waveRaf = 0;
      const bytes = bytesSinceFrame;
      bytesSinceFrame = 0;
      smoothed = Math.max(levelFromBytes(bytes), smoothed * RELEASE);
      if (smoothed < 0.004) smoothed = 0;
      history.push(smoothed);
      if (history.length > HISTORY) history.shift();
      if (host.clientWidth >= 20 && host.clientHeight >= 20) {
        drawWave();
        drawArc();
      }
      // Keep animating until the last pulse has shifted out of the window.
      if (smoothed > 0 || history.some((v) => v > 0.004)) {
        waveRaf = requestAnimationFrame(waveTick);
      } else {
        waveRunning = false;
        history.length = 0;
        for (const c of [waveCanvas, arcCanvas])
          c?.getContext("2d")?.clearRect(0, 0, c.width, c.height);
      }
    }

    // 시스템의 "애니메이션 사용" 설정은 여기서 보지 않는다 — 위 .wave-lab 주석 참고.
    function waveWake() {
      if (waveRunning) return;
      waveRunning = true;
      waveRaf = requestAnimationFrame(waveTick);
    }

    readAccent();
    const channel = subscribePane(pane, (chunk) => {
      term.write(chunk);
      bytesSinceFrame += chunk.length; // byte count only — never the content
      waveWake();
    });
    term.onData((data) => {
      clearKeyboardSelection();
      trackInput(data);
      void writePane(pane, data);
      // Broadcast (synchronize-panes): mirror this pane's input to every other
      // live pane in the workspace. Only the focused pane originates; writePane
      // feeds the PTY (not xterm.onData), so mirrored panes never echo back.
      if (broadcast.on && focused) {
        for (const target of broadcastTargets(pane)) void writePane(target, data);
      }
    });

    let resizeRaf = 0;
    let wasHidden = true;
    const doFit = () => {
      // Hidden workspaces report 0×0; fitting then would shrink the PTY to
      // a few columns and garble every TUI in the pane. Skip until visible.
      if (host.clientWidth < 20 || host.clientHeight < 20) {
        wasHidden = true;
        return;
      }
      fit.fit();
      if (term.cols >= 2 && term.rows >= 2) {
        void resizePane(pane, term.cols, term.rows);
      }
      if (wasHidden) {
        // Coming back from display:none the canvas is stale/blank and a
        // same-size fit() is a no-op, so nothing would repaint it.
        wasHidden = false;
        term.refresh(0, term.rows - 1);
      }
    };
    refit = doFit;
    const observer = new ResizeObserver(() => {
      cancelAnimationFrame(resizeRaf);
      resizeRaf = requestAnimationFrame(doFit);
    });
    observer.observe(host);
    doFit();
    if (focused) term.focus();
    // Don't yank the keyboard out of the composer: focusTerm() fires on every
    // active-pane change (snapshot listener, palette close, sidebar click), and
    // clicking this pane's composer is exactly such a change.
    const unregisterFocus = registerTermFocus(pane, () => {
      // 조합 중이면 포커스를 옮기지 않는다 — 옮기는 순간 xterm의 조합 버퍼가
      // blur로 비워지면서 이미 친 글이 중복되거나 사라진다.
      if (!composerFocused && !imeComposing) term.focus();
    });
    const unregisterComposer = registerComposerFocus(pane, () => {
      if (!composerOn) {
        setShowComposer(true);
        void tick().then(() => ta?.focus());
      } else if (composerFocused) {
        term.focus();
      } else {
        ta?.focus();
      }
    });

    return () => {
      unregisterComposer();
      unregisterFocus();
      observer.disconnect();
      cancelAnimationFrame(waveRaf);
      host.removeEventListener("keydown", stampKey, true);
      host.removeEventListener("compositionstart", resetImeBuffer, true);
      xtermTextarea?.removeEventListener("compositionstart", imeOn);
      xtermTextarea?.removeEventListener("compositionend", imeOff);
      channel.onmessage = () => {};
      term.dispose();
    };
  });

  $effect(() => {
    // untrack: 입력창에서 포커스가 *빠질* 때 이 effect가 다시 돌아 터미널로
    // 포커스를 뺏어오면 안 된다(예: 사이드바 입력창을 클릭한 경우).
    // imeComposing은 일부러 추적한다 — 조합 중에는 건너뛰었다가, 조합이
    // 끝나 false가 되는 순간 이 effect가 다시 돌면서 포커스를 마저 옮긴다.
    if (imeComposing) return;
    if (focused && term && !untrack(() => composerFocused)) term.focus();
  });

  // Live font-size changes: update xterm, then refit cols/rows to the host.
  $effect(() => {
    const size = settings.fontSize;
    if (term && term.options.fontSize !== size) {
      term.options.fontSize = size;
      refit?.();
    }
  });

  // Live theme changes.
  $effect(() => {
    const theme = themeById(settings.theme).term;
    if (term) term.options.theme = theme;
  });

  // Thinking Waveform crest follows the active theme; re-read after the next
  // frame so the theme's CSS vars are already applied to :root.
  $effect(() => {
    void settings.theme;
    requestAnimationFrame(readAccent);
  });

  async function copySelection() {
    const sel = term.getSelection();
    if (sel) await writeText(sel);
  }

  async function pasteClipboard() {
    try {
      const text = await readText();
      // term.paste() honors bracketed-paste mode (vim, fzf, modern shells)
      // and feeds onData → PTY.
      if (text) term.paste(text);
    } catch {
      // Clipboard empty or unreadable — nothing to paste.
    }
  }

  async function menuAction(action: "copy" | "cut" | "paste" | "selectAll" | "clear") {
    menu = null;
    switch (action) {
      case "copy":
        await copySelection();
        break;
      case "cut":
        await cutSelection();
        break;
      case "paste":
        await pasteClipboard();
        break;
      case "selectAll":
        term.selectAll();
        break;
      case "clear":
        term.clear();
        break;
    }
    term.focus();
  }
</script>

<svelte:window onclick={() => (menu = null)} />

<!-- 터미널 + 하단 입력창을 세로로 쌓는 스택. 입력창을 켜면 터미널이 그만큼
     짧아지므로(PTY도 함께 리사이즈) 가려지는 내용이 없다. -->
<div class="term-stack" style="--cfs: {settings.fontSize}px; --clh: {Math.round(settings.fontSize * 1.45)}px">
<div
  class="terminal-host"
  role="application"
  bind:this={host}
  oncontextmenu={(e) => {
    e.preventDefault();
    menu = { x: e.clientX, y: e.clientY };
  }}
  onwheel={(e) => {
    // Ctrl+wheel zooms the font, like GNOME Terminal.
    if (e.ctrlKey) {
      e.preventDefault();
      adjustFontSize(e.deltaY < 0 ? 1 : -1);
    }
  }}
  onauxclick={(e) => {
    // Middle-click pastes, like every Linux terminal.
    if (e.button === 1) {
      e.preventDefault();
      void pasteClipboard();
    }
  }}
  onpastecapture={(e) => {
    // 네이티브 paste 경로 차단 — 붙여넣기는 항상 우리(Rust 클립보드)
    // 경로 하나로만 들어와 이중 붙여넣기를 방지.
    e.preventDefault();
    e.stopPropagation();
  }}
></div>
  {#if composerOn}
    <div
      class="composer"
      class:active={composerFocused}
      class:waiting={waitingInput}
    >
      <textarea
        bind:this={ta}
        bind:value={draft}
        rows="2"
        spellcheck="false"
        placeholder={waitingInput
          ? "⌨ 터미널이 선택을 기다립니다 — 위쪽 화면에서 응답하세요"
          : "프롬프트 입력 — Enter 전송 · / 는 터미널로"}
        title="Enter 전송 · Shift+Enter 줄바꿈 · Ctrl+Enter 제출 없이 입력만 · Esc 터미널로 (Ctrl+Shift+E 로 오가기)
빈 칸에서 / ! # @ 와 ↑ ↓ Tab 은 터미널로 바로 넘어갑니다 (슬래시 명령 자동완성 · 옵션 선택)"
        oninput={autosize}
        onkeydown={composerKey}
        onfocus={() => (composerFocused = true)}
        onblur={() => (composerFocused = false)}
        oncompositionstart={() => (imeComposing = true)}
        oncompositionend={() => (imeComposing = false)}
      ></textarea>
      <button
        class="send"
        title="전송 (Enter)"
        disabled={draft.trim() === ""}
        onmousedown={(e) => e.preventDefault()}
        onclick={() => void sendDraft(true)}>⏎</button
      >
    </div>
  {/if}
</div>

{#if settings.showLastInput && lastInput}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="last-input"
    class:dragging
    bind:this={bannerEl}
    aria-hidden="true"
    title="드래그해서 위치 이동"
    style="left: {settings.lastInputPos?.x ?? 8}px; top: {settings.lastInputPos?.y ?? 6}px"
    onpointerdown={bannerPointerDown}
    onpointermove={bannerPointerMove}
    onpointerup={bannerPointerUp}
    onpointercancel={bannerPointerUp}
  >
    <span class="li-label"><span class="li-grip">⠿</span> 직전 명령</span>
    <span class="li-text" style="font-size: {settings.fontSize}px">{lastInput}</span>
  </div>
{/if}

<div class="wave-lab" aria-hidden="true">
  <canvas class="wl wave" bind:this={waveCanvas}></canvas>
  <canvas class="wl arc" bind:this={arcCanvas}></canvas>
</div>

{#if searchOpen}
  <div class="search-bar">
    <!-- svelte-ignore a11y_autofocus -->
    <input
      autofocus
      placeholder="검색…"
      bind:value={searchQuery}
      oninput={() => search.findNext(searchQuery, { ...searchOptions, incremental: true })}
      onkeydown={(e) => {
        e.stopPropagation();
        if (e.key === "Enter" && e.shiftKey) search.findPrevious(searchQuery, searchOptions);
        else if (e.key === "Enter") search.findNext(searchQuery, searchOptions);
        else if (e.key === "Escape") closeSearch();
      }}
    />
    <button title="이전 (Shift+Enter)" onclick={() => search.findPrevious(searchQuery, searchOptions)}>↑</button>
    <button title="다음 (Enter)" onclick={() => search.findNext(searchQuery, searchOptions)}>↓</button>
    <button title="닫기 (Esc)" onclick={closeSearch}>✕</button>
  </div>
{/if}

{#if menu}
  <div class="ctx-menu" style="left: {menu.x}px; top: {menu.y}px">
    <button onclick={() => menuAction("copy")} disabled={!term?.hasSelection()}>복사</button>
    <button onclick={() => menuAction("cut")} disabled={!term?.hasSelection()}>잘라내기</button>
    <button onclick={() => menuAction("paste")}>붙여넣기</button>
    <button onclick={() => menuAction("selectAll")}>모두 선택</button>
    <button onclick={() => menuAction("clear")}>화면 지우기</button>
    {#if extraActions.length > 0}
      <hr />
      {#each extraActions as action (action.label)}
        <button
          onclick={() => {
            menu = null;
            action.run();
          }}>{action.label}</button
        >
      {/each}
    {/if}
  </div>
{/if}

<style>
  .term-stack {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg);
  }
  .terminal-host {
    flex: 1 1 auto;
    width: 100%;
    min-height: 0;
    background: var(--bg);
  }
  /* 하단 고정 입력창 — 터미널과 완전히 분리된 입력칸이라 여기 타이핑해도
     xterm의 스크롤 위치가 움직이지 않는다. 세로 스택 안에 그대로 놓여 있으므로
     (position: absolute 로 띄우지 않는다) 입력창이 길어지면 그만큼 터미널 칸이
     실제로 줄어든다 — 터미널 아래가 가려지는 일이 없다. 줄어든 칸은
     ResizeObserver가 감지해 xterm과 PTY 크기까지 함께 맞춘다. */
  .composer {
    flex: 0 0 auto;
    display: flex;
    align-items: flex-end;
    gap: 6px;
    padding: 5px 6px;
    background: var(--surface-2);
    border-top: 1px solid var(--border-2);
  }
  .composer.active {
    border-top-color: var(--accent);
  }
  /* Claude가 권한/선택을 묻는 중 — 키보드는 터미널 차례라는 신호. */
  .composer.waiting {
    border-top-color: var(--yellow);
    background: color-mix(in srgb, var(--yellow) 8%, var(--surface-2));
  }
  .composer.waiting textarea::placeholder {
    color: var(--yellow);
  }
  .composer textarea {
    flex: 1;
    min-width: 0;
    /* 쉴 때 항상 2줄 — autosize가 언제 돌든 예약 높이가 흔들리지 않는다. */
    min-height: calc(2 * var(--clh) + 8px);
    padding: 3px 6px;
    font-family: monospace;
    font-size: var(--cfs);
    line-height: var(--clh);
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--border-2);
    border-radius: 5px;
    resize: none;
    overflow-y: auto;
    white-space: pre-wrap;
    word-break: break-word;
    /* body가 user-select:none이라 상속되면 입력 텍스트 선택이 막힌다. */
    user-select: text;
  }
  .composer textarea:focus {
    outline: none;
    border-color: var(--accent);
  }
  .composer textarea::placeholder {
    color: var(--muted);
  }
  .composer .send {
    flex-shrink: 0;
    width: 28px;
    height: 26px;
    color: var(--text);
    background: none;
    border: 1px solid var(--border-2);
    border-radius: 5px;
    cursor: pointer;
    font-size: 0.85rem;
    line-height: 1;
  }
  .composer .send:hover:not(:disabled) {
    background: var(--accent);
    color: var(--bg);
  }
  .composer .send:disabled {
    opacity: 0.35;
    cursor: default;
  }
  /* Last-command chip — the user's most recent submitted command, floating
     inside the pane as a draggable chip. Overlay only (never resizes the PTY).
     Position comes from settings (inline left/top); clamped to 4 lines. */
  .last-input {
    position: absolute;
    z-index: 3;
    width: fit-content;
    max-width: min(520px, calc(100% - 16px));
    pointer-events: auto;
    cursor: grab;
    touch-action: none;
    user-select: none;
    padding: 3px 10px 5px;
    background: color-mix(in srgb, var(--surface-2) 96%, transparent);
    border: 1px solid color-mix(in srgb, var(--accent) 45%, transparent);
    border-radius: 7px;
    box-shadow: 0 3px 12px -3px rgba(0, 0, 0, 0.5);
  }
  .last-input.dragging {
    cursor: grabbing;
    box-shadow: 0 6px 20px -4px rgba(0, 0, 0, 0.6);
  }
  .li-label {
    display: block;
    font-size: 0.6rem;
    font-weight: 700;
    letter-spacing: 0.03em;
    color: var(--accent);
    opacity: 0.85;
    margin-bottom: 1px;
  }
  .li-grip {
    color: var(--muted);
    opacity: 0.8;
  }
  .li-text {
    display: -webkit-box;
    -webkit-line-clamp: 4;
    line-clamp: 4;
    -webkit-box-orient: vertical;
    overflow: hidden;
    /* font-size is set inline to track the terminal (prompt) font size. */
    line-height: 1.35;
    color: var(--text-2);
    white-space: pre-wrap;
    word-break: break-word;
  }
  /* Activity widgets — the oscilloscope waveform (left) paired with the Arc
     Reactor core (right), in the pane's top-right corner. Above the terminal,
     below the hover toolbar/shockwave; never eat clicks. Both adapt in JS. */
  .wave-lab {
    position: absolute;
    top: 6px;
    right: 8px;
    z-index: 4;
    pointer-events: none;
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 8px;
  }
  .wl {
    display: block;
  }
  .wl.wave {
    width: 140px;
    height: 46px;
  }
  .wl.arc {
    width: 48px;
    height: 46px;
  }
  /* 여기 있던 `@media (prefers-reduced-motion: reduce) { .wave-lab { display: none } }`
     는 일부러 뺐다. GNOME의 "애니메이션 사용"을 끄면(gsettings의
     org.gnome.desktop.interface enable-animations = false) GTK/WebKitGTK가 그것을
     그대로 이 미디어 질의로 전달하는데, 그러면 파형이 통째로 사라져 "출력이 흐르고
     있다"는 정보까지 같이 없어졌다. 실제로 그 설정이 꺼져 있던 사용자가 위젯이
     사라진 것으로 겪었다. amux는 시스템 애니메이션 설정과 무관하게 동작한다는 것이
     의도한 방침이므로, 이 파일과 App.svelte·PaneView.svelte·Dashboard.svelte의
     같은 규칙을 모두 제거했다. 되돌리지 말 것. */
  .ctx-menu {
    position: fixed;
    z-index: 1000;
    display: flex;
    flex-direction: column;
    min-width: 10rem;
    padding: 0.25rem;
    background: var(--surface-2);
    border: 1px solid var(--border-2);
    border-radius: 6px;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.5);
  }
  .ctx-menu button {
    padding: 0.4rem 0.75rem;
    text-align: left;
    color: var(--text);
    background: none;
    border: none;
    border-radius: 4px;
    cursor: pointer;
    font-size: 0.85rem;
  }
  .ctx-menu button:hover:not(:disabled) {
    background: var(--border-2);
  }
  .ctx-menu button:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .ctx-menu hr {
    margin: 0.25rem 0.5rem;
    border: none;
    border-top: 1px solid var(--border-2);
  }
  .search-bar {
    position: absolute;
    top: 4px;
    right: 110px;
    z-index: 20;
    display: flex;
    gap: 2px;
    padding: 3px;
    background: var(--surface-2);
    border: 1px solid var(--border-2);
    border-radius: 6px;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.5);
  }
  .search-bar input {
    width: 11rem;
    padding: 2px 6px;
    font-size: 0.8rem;
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--border-2);
    border-radius: 4px;
  }
  .search-bar input:focus {
    outline: none;
    border-color: var(--accent);
  }
  .search-bar button {
    width: 22px;
    color: var(--text);
    background: none;
    border: none;
    border-radius: 4px;
    cursor: pointer;
  }
  .search-bar button:hover {
    background: var(--border-2);
  }
</style>
