<script lang="ts">
  // One subagent's card: its task, what it is doing now, every step it took
  // (files read, commands run, the change each edit made) and its final
  // report. Read-only: it shows what the transcript on disk says.
  import { tick } from "svelte";
  import { toolSummary } from "./format";
  import { renderMarkdown } from "./markdown";
  import { activity, running, stepDiff, type SubAgent } from "./subagents";

  let { agent, cwd, onclose }: { agent: SubAgent; cwd: string | null; onclose: () => void } = $props();

  const live = $derived(running(agent));
  const now = $derived(live ? activity(agent, cwd) : null);
  const steps = $derived(agent.rows.filter((r) => r.kind === "tool").length);

  /** Lines of a change shown before "더 보기". */
  const DIFF_LINES = 24;
  let openDiff = $state<Record<string, boolean>>({});
  let taskOpen = $state(false);

  // Follow the bottom while it is there; scrolling up to read stops that.
  // When the report comes, show where it starts (it may be long).
  let logEl = $state<HTMLElement | null>(null);
  let reportEl = $state<HTMLElement | null>(null);
  let stick = true;
  let reportShown = false;
  function onScroll() {
    if (!logEl) return;
    stick = logEl.scrollHeight - logEl.scrollTop - logEl.clientHeight < 24;
  }
  $effect(() => {
    void agent.rows.length;
    void steps;
    const report = agent.report;
    void tick().then(() => {
      if (!stick || !logEl) return;
      if (report && !reportShown && reportEl) {
        reportShown = true;
        logEl.scrollTop += reportEl.getBoundingClientRect().top - logEl.getBoundingClientRect().top - 6;
      } else if (!report) logEl.scrollTop = logEl.scrollHeight;
    });
  });
</script>

<article class="card" class:live class:stopped={agent.stopped} data-agent={agent.id}>
  <header>
    <span class="no">서브 {agent.no}</span>
    <span class="desc" title={agent.description ?? ""}>{agent.description ?? "서브에이전트"}</span>
    {#if agent.type}<span class="type">{agent.type}</span>{/if}
    <span class="state">
      {#if live}<span class="spin"></span> 작업 중{:else if agent.stopped}■ 멈춤{:else}✓ 완료{/if}
    </span>
    {#if !live}
      <button class="x" title="이 창 닫기 (다음 메시지를 보내면 저절로 닫힙니다)" onclick={onclose}>✕</button>
    {/if}
  </header>
  {#if live}
    <div class="now" title={now ?? ""}>지금: {now ?? "생각하는 중"}</div>
  {/if}
  <div class="log" bind:this={logEl} onscroll={onScroll}>
    {#each agent.rows as r, i (i)}
      {#if r.kind === "task"}
        <button class="task" class:open={taskOpen} title={taskOpen ? "접기" : "펼치기"} onclick={() => (taskOpen = !taskOpen)}>
          <span class="label">맡은 일</span>
          <span class="body">{r.text}</span>
        </button>
      {:else if r.kind === "say"}
        <div class="say"><span class="label">보낸 말</span> {r.text}</div>
      {:else if r.kind === "text"}
        <div class="md text">{@html renderMarkdown(r.text)}</div>
      {:else if r.kind === "tool"}
        {@const t = agent.tools[r.id]}
        {#if t}
          {@const diff = stepDiff(t)}
          <div class="tool" class:err={t.err}>
            <span class="mark">
              {#if !t.done}{#if live}<span class="spin"></span>{:else}■{/if}{:else if t.err}✗{:else}✓{/if}
            </span>
            <span class="name">{t.name}</span>
            <span class="sum" title={toolSummary(t.name, t.input, cwd)}>{toolSummary(t.name, t.input, cwd)}</span>
          </div>
          {#if diff && diff.length}
            {@const all = openDiff[t.id] === true}
            <div class="diff">
              {#each all ? diff : diff.slice(0, DIFF_LINES) as l, j (j)}
                {#if l.kind === "gap"}
                  <div class="gap">⋯</div>
                {:else}
                  <div class="dl {l.kind}">
                    <span class="sign">{l.kind === "add" ? "+" : l.kind === "del" ? "-" : ""}</span><span
                      class="dt">{l.text}</span>
                  </div>
                {/if}
              {/each}
              {#if diff.length > DIFF_LINES}
                <button class="more" onclick={() => (openDiff[t.id] = !all)}>
                  {all ? "줄이기" : `${diff.length - DIFF_LINES}줄 더 보기`}
                </button>
              {/if}
            </div>
          {/if}
          {#if t.err && t.result}
            <div class="errtext">{t.result}</div>
          {/if}
        {/if}
      {/if}
    {/each}
    {#if agent.report}
      <section class="report" bind:this={reportEl}>
        <div class="label">보고</div>
        <div class="md">{@html renderMarkdown(agent.report)}</div>
      </section>
    {:else if !live && steps === 0 && agent.rows.length <= 1}
      <div class="empty">기록이 없습니다.</div>
    {/if}
  </div>
</article>

<style>
  .card {
    flex: 1 1 0;
    min-height: 150px;
    display: flex;
    flex-direction: column;
    border-bottom: 1px solid var(--border);
    color: var(--text);
  }
  .card:last-child {
    border-bottom: none;
  }
  header {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 5px 8px;
    background: var(--surface-2);
    border-bottom: 1px solid var(--border);
    font-size: 0.86em;
    min-width: 0;
  }
  .card.live header {
    box-shadow: inset 3px 0 0 var(--info);
  }
  .no {
    flex: none;
    font-weight: 600;
    color: var(--accent);
  }
  .desc {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .type {
    flex: none;
    color: var(--muted);
    border: 1px solid var(--border-2);
    border-radius: 8px;
    padding: 0 6px;
    font-size: 0.9em;
  }
  .state {
    flex: none;
    color: var(--green);
    white-space: nowrap;
  }
  .card.live .state {
    color: var(--info);
  }
  .card.stopped .state {
    color: var(--yellow);
  }
  .x {
    flex: none;
    font: inherit;
    color: var(--muted);
    background: none;
    border: none;
    padding: 0 2px;
    cursor: pointer;
  }
  .x:hover {
    color: var(--text);
  }
  .now {
    padding: 3px 8px;
    font-size: 0.84em;
    color: var(--info);
    border-bottom: 1px solid var(--border);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .log {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    padding: 6px 8px 10px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 0.9em;
    line-height: 1.45;
  }
  /* The log scrolls as a whole; a row that scrolls sideways (a diff, a table)
     must not be squeezed shorter than its content to make room. */
  .log > :global(*) {
    flex: none;
  }
  .label {
    display: inline-block;
    font-size: 0.85em;
    font-weight: 600;
    color: var(--muted);
    margin-right: 4px;
  }
  .task {
    display: block;
    text-align: left;
    font: inherit;
    color: inherit;
    background: none;
    border: 1px solid var(--border);
    border-radius: 5px;
    padding: 4px 6px;
    cursor: pointer;
  }
  .task .body {
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
    white-space: pre-wrap;
    word-break: break-word;
    color: var(--text-2, var(--text));
  }
  .task.open .body {
    display: block;
  }
  .say {
    white-space: pre-wrap;
    word-break: break-word;
    border-left: 3px solid var(--border-2);
    padding-left: 6px;
  }
  .tool {
    display: flex;
    align-items: baseline;
    gap: 6px;
    min-width: 0;
    font-size: 0.95em;
  }
  .tool .mark {
    flex: none;
    width: 1em;
    text-align: center;
    color: var(--green);
  }
  .tool.err .mark {
    color: var(--red);
  }
  .tool .name {
    flex: none;
    font-weight: 600;
  }
  .tool .sum {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: ui-monospace, "JetBrains Mono", "D2Coding", monospace;
    font-size: 0.92em;
    color: var(--text-2, var(--muted));
  }
  .errtext {
    margin-left: 1.5em;
    color: var(--red);
    font-size: 0.88em;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .diff {
    margin-left: 1.5em;
    font-family: ui-monospace, "JetBrains Mono", "D2Coding", monospace;
    font-size: 0.84em;
    line-height: 1.4;
    background: var(--surface-2);
    border-radius: 4px;
    overflow-x: auto;
  }
  .dl {
    display: grid;
    grid-template-columns: 1.2em minmax(0, 1fr);
    /* An empty line is still a line. */
    min-height: 1.4em;
  }
  .dl.add {
    background: color-mix(in srgb, var(--green) 15%, transparent);
  }
  .dl.del {
    background: color-mix(in srgb, var(--red) 15%, transparent);
  }
  .sign {
    text-align: center;
    user-select: none;
    color: var(--muted);
  }
  .dl.add .sign {
    color: var(--green);
  }
  .dl.del .sign {
    color: var(--red);
  }
  .dt {
    white-space: pre-wrap;
    word-break: break-all;
    padding-right: 6px;
  }
  .gap {
    color: var(--muted);
    text-align: center;
    font-size: 0.85em;
    border-top: 1px dashed var(--border);
  }
  .more {
    display: block;
    width: 100%;
    font: inherit;
    color: var(--info);
    background: none;
    border: none;
    padding: 3px;
    cursor: pointer;
  }
  .report {
    margin-top: 4px;
    border: 1px solid color-mix(in srgb, var(--green) 45%, var(--border));
    border-radius: 6px;
    padding: 4px 8px 6px;
    background: color-mix(in srgb, var(--green) 6%, transparent);
  }
  .empty {
    color: var(--muted);
    font-size: 0.9em;
  }
  .md {
    word-break: break-word;
  }
  .md :global(p) {
    margin: 0.3em 0;
  }
  .md :global(:is(h1, h2, h3, h4)) {
    margin: 0.5em 0 0.25em;
    font-size: 1.02em;
    line-height: 1.3;
  }
  .md :global(:is(ul, ol)) {
    margin: 0.25em 0;
    padding-left: 1.3em;
  }
  .md :global(code) {
    font-family: ui-monospace, "JetBrains Mono", "D2Coding", monospace;
    font-size: 0.9em;
    background: var(--surface-2);
    border-radius: 4px;
    padding: 0.05em 0.3em;
  }
  .md :global(pre) {
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 5px;
    padding: 6px 8px;
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
    padding: 2px 6px;
  }
  .md :global(a) {
    color: var(--info);
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
</style>
