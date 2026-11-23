<script lang="ts">
  // The "직전 명령" strip: a pane's most recently sent command or prompt as a
  // full-width strip, which unfolds into every one sent so far. Shared by the
  // terminal (Terminal.svelte, above xterm) and the chat view (ChatView.svelte,
  // under its header): both read and add to the same per-pane log in
  // state.svelte's `paneCommands`, so a pane shows one history whichever mode
  // it is in. Hidden while the log is empty or the ⚙ setting is off.
  import type { PaneId } from "./ipc";
  import { settings } from "./settings.svelte";
  import { paneCommands } from "./state.svelte";

  let {
    pane,
    areaHeight = 0,
    rightPad = "8px",
  }: {
    pane: PaneId;
    /** Height of the area the unfolded log hangs over (the pane), for its cap. */
    areaHeight?: number;
    /** Room kept clear at the right end (the terminal's activity widgets float there). */
    rightPad?: string;
  } = $props();

  const cmdLog = $derived(paneCommands.log[pane] ?? []);
  const lastInput = $derived(cmdLog[cmdLog.length - 1] ?? "");

  // ── 명령 기록 펼치기 ─────────────────────────────────────────────────────
  // 접힌 상태(기본)는 직전 명령 한 줄만 보여 준다. 띠를 클릭하면 그 아래로
  // 기록 목록이 펼쳐지고(최신이 위), 다시 클릭하면 접힌다.
  //
  // 펼친 목록은 세로 스택의 한 행이 아니라 **떠 있는 패널**이다(띠를 감싼
  // .last-input-wrap 을 기준으로 absolute, 그 바로 아래에 매달린다). 행으로
  // 넣으면 펼칠 때마다 아래 칸이 줄어든다. 터미널이라면 ResizeObserver 가 그걸
  // 감지해 PTY까지 리사이즈하므로 안에서 돌던 Claude Code가 열 때 한 번, 닫을 때
  // 한 번 화면을 다시 그린다. 자주 누르는 토글이 매번 화면을 흔들면 안 되니 떠
  // 있게 두고 아래 칸의 크기는 건드리지 않는다.
  let logOpen = $state(false);
  // 패널 높이 상한은 **pane 높이**의 45% 여야 한다. 45vh(창 높이 기준)로 잡으면
  // 위아래로 분할한 짧은 pane에서 패널이 pane 밖까지 흘러내린다.
  const logMaxH = $derived(Math.max(120, Math.round((areaHeight || 400) * 0.45)));
  function toggleLog() {
    if (cmdLog.length === 0) return;
    logOpen = !logOpen;
  }
  // 펼친 목록은 최신이 위: 접힌 띠에 있던 직전 명령이 그 자리에 그대로 남아
  // 있고 그 아래로 옛 명령이 이어지므로, 눈이 따라갈 곳을 잃지 않는다.
  const cmdLogNewestFirst = $derived([...cmdLog].reverse());
</script>

{#if settings.showLastInput && lastInput}
  <div class="last-input-wrap" style="--clh: {Math.round(settings.fontSize * 1.45)}px; --li-right: {rightPad}">
    <button
      type="button"
      class="last-input"
      class:open={logOpen}
      aria-expanded={logOpen}
      title={logOpen
        ? "클릭해 접기 — 직전 명령 한 줄만 보이기"
        : `클릭해 이 pane에 보낸 명령 ${cmdLog.length}개 모두 보기\n\n${lastInput}`}
      onmousedown={(e) => e.preventDefault()}
      onclick={toggleLog}
    >
      <span class="li-label"
        ><span class="li-caret">{logOpen ? "▾" : "▸"}</span>{logOpen ? "명령 기록" : "직전 명령"}</span
      >
      <span class="li-text" style="font-size: {settings.fontSize}px">{lastInput}</span>
      <span class="li-count">{cmdLog.length}</span>
    </button>
    {#if logOpen}
      <div class="li-log" role="list" style="max-height: {logMaxH}px">
        {#each cmdLogNewestFirst as cmd, i (cmdLog.length - i)}
          <div class="li-row" class:newest={i === 0} role="listitem">
            <span class="li-no">{cmdLog.length - i}</span>
            <span class="li-cmd" style="font-size: {settings.fontSize}px">{cmd}</span>
          </div>
        {/each}
      </div>
    {/if}
  </div>
{/if}

<style>
  /* The strip is a ROW OF ITS PARENT'S FLEX STACK (not an overlay), so it never
     covers what is under it: in the terminal, xterm shrinks by exactly this
     strip's height and the ResizeObserver refits xterm and the PTY to match.
     Hard newlines the user typed are kept (white-space: pre, capped at 4
     lines); soft wrapping is off, so anything past the right edge is
     ellipsized (hover for the full text). In the terminal the right padding
     (--li-right, 210px there) keeps the text clear of .wave-lab, which floats
     over this strip's right end (140px wave + 8px gap + 48px arc + 8px inset
     ≈ 204px). */
  .last-input-wrap {
    /* 접힌 띠만 세로 스택의 한 행이다. 펼친 목록(.li-log)은 이 래퍼를 기준으로
       떠 있어서(absolute) 아래 칸의 높이를 건드리지 않는다.
       래퍼 자체에는 z-index를 주지 않는다: 주면 쌓임 맥락(stacking context)이
       생겨 **접힌 띠까지** 위로 올라가고, 그러면 터미널 오른쪽 위에 떠 있는 파형
       위젯(.wave-lab, z-index 4)의 윗부분이 이 띠의 불투명 배경에 가린다.
       그래서 z-index 는 펼친 패널에만 준다. */
    position: relative;
    flex: 0 0 auto;
  }
  .last-input {
    width: 100%;
    display: flex;
    align-items: baseline;
    gap: 8px;
    /* 쉴 때 항상 1줄: 첫 명령이 들어와도 아래 칸 높이가 튀지 않는다. */
    min-height: calc(var(--clh) + 8px);
    padding: 3px var(--li-right) 4px 8px;
    background: color-mix(in srgb, var(--surface-2) 96%, transparent);
    border: none;
    border-bottom: 1px solid color-mix(in srgb, var(--accent) 45%, transparent);
    user-select: none;
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .last-input:hover {
    background: color-mix(in srgb, var(--surface-2) 70%, var(--border-2));
  }
  .last-input.open {
    background: color-mix(in srgb, var(--surface-2) 60%, var(--border-2));
    border-bottom-color: var(--accent);
  }
  .li-caret {
    display: inline-block;
    width: 0.7em;
    margin-right: 2px;
  }
  /* 기록 개수: 펼치면 몇 개가 나오는지 누르기 전에 알 수 있게. */
  .li-count {
    flex: 0 0 auto;
    align-self: center;
    padding: 0 5px;
    border-radius: 7px;
    background: color-mix(in srgb, var(--accent) 22%, transparent);
    color: var(--accent);
    font-size: 0.6rem;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }
  /* 펼친 명령 기록: 최신이 맨 위. pane 높이의 45% 를 넘지 않고 그 안에서
     스크롤된다. 행은 복사할 수 있어야 하니 부모의 user-select: none 을 되돌린다. */
  .li-log {
    position: absolute;
    top: 100%;
    left: 0;
    right: 0;
    /* 7 = 파형 위젯(.wave-lab 4)과 완료 충격파(PaneView의 .fx 6) 보다 위,
       호버 툴바(10)와 드래그 오버레이(15) 보다 아래. 래퍼가 쌓임 맥락을 만들지
       않으므로 부모 쪽 층과 같은 자리에서 겨룬다. */
    z-index: 7;
    /* max-height is set inline from the pane's own height (see logMaxH). */
    overflow-y: auto;
    overscroll-behavior: contain;
    background: var(--surface-2);
    border-bottom: 1px solid var(--accent);
    box-shadow: 0 8px 20px rgba(0, 0, 0, 0.45);
    user-select: text;
  }
  .li-row {
    display: flex;
    align-items: baseline;
    gap: 8px;
    padding: 3px 8px 4px;
    border-top: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
  }
  .li-row:first-child {
    border-top: none;
  }
  .li-row.newest {
    background: color-mix(in srgb, var(--accent) 9%, transparent);
  }
  .li-no {
    flex: 0 0 auto;
    min-width: 1.6em;
    text-align: right;
    color: var(--accent);
    opacity: 0.7;
    font-size: 0.6rem;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }
  .li-cmd {
    flex: 1 1 auto;
    min-width: 0;
    line-height: 1.35;
    color: var(--text-2);
    /* 목록은 세로로 스크롤되니, 긴 줄은 잘라 버리지 말고 접어서 다 보여 준다.
       다만 한 항목이 패널을 다 먹지 않도록 8줄에서 멈춘다. */
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    overflow: hidden;
    max-height: calc(8 * 1.35em);
  }
  .li-label {
    flex: 0 0 auto;
    font-size: 0.6rem;
    font-weight: 700;
    letter-spacing: 0.03em;
    color: var(--accent);
    opacity: 0.85;
  }
  .li-text {
    flex: 1 1 auto;
    min-width: 0;
    /* font-size is set inline to track the terminal (prompt) font size. */
    line-height: 1.35;
    color: var(--text-2);
    /* 내가 친 개행(Shift+Enter)은 그대로 살리고, 폭이 좁아서 생기는 자동
       줄바꿈만 막는다. 한 줄이 폭을 넘치면 끝을 … 로 자른다. */
    white-space: pre;
    overflow: hidden;
    text-overflow: ellipsis;
    /* 아주 긴 여러 줄 프롬프트가 아래 칸 높이를 다 먹지 않도록 4줄에서 멈춘다.
       (1.35 는 바로 위 line-height) */
    max-height: calc(4 * 1.35em);
  }
</style>
