<script lang="ts">
  // A file edit in the conversation, shown live. While Claude is still
  // writing the tool call, the old text (red) and the new text (green) grow as
  // they arrive; once applied, the card turns into the real diff Claude Code
  // reports for it (`structuredPatch`), with line numbers.
  import type { ChatItem } from "./reducer";
  import { partialFields } from "./partialJson";
  import { addedHunk, countChanges, type Hunk } from "./diff";
  import { clip, shortPath } from "./format";
  import DiffHunks from "./DiffHunks.svelte";

  type ToolItem = Extract<ChatItem, { kind: "tool" }>;
  let {
    item,
    base = null,
    compact = false,
  }: {
    item: ToolItem;
    /** Paths inside this folder show relative. */
    base?: string | null;
    /** Unused: the card now starts folded wherever the files panel is
     *  (beside the conversation, in the pop-out window, or closed). */
    compact?: boolean;
  } = $props();

  // Re-reading the growing JSON on every piece would cost O(n²) for a long
  // Write, so read it at most ten times a second.
  let live = $state<Record<string, string>>({});
  let timer: ReturnType<typeof setTimeout> | null = null;
  $effect(() => {
    const raw = item.inputJson;
    // Folded: nothing of the text is shown, so it is not read either.
    if (!open || item.inputDone || !raw || timer) return;
    timer = setTimeout(() => {
      timer = null;
      const fields = partialFields(item.inputJson);
      live = Object.fromEntries(Object.entries(fields).map(([k, f]) => [k, f.value]));
    }, 100);
  });
  $effect(() => () => {
    if (timer) clearTimeout(timer);
  });

  function stringFields(input: Record<string, unknown>): Record<string, string> {
    const out: Record<string, string> = {};
    for (const [k, v] of Object.entries(input)) if (typeof v === "string") out[k] = v;
    return out;
  }

  const fields = $derived(item.inputDone ? stringFields(item.input) : live);
  const path = $derived(item.path ?? fields.file_path ?? fields.notebook_path ?? "");
  const phase = $derived(
    item.done
      ? item.isError
        ? "error"
        : "applied"
      : item.awaiting
        ? "awaiting"
        : item.inputDone
          ? "running"
          : "writing",
  );
  const PHASE_LABEL: Record<string, string> = {
    writing: "작성 중",
    awaiting: "승인 대기",
    running: "적용 중",
    applied: "적용됨",
    error: "실패",
  };
  const label = $derived(
    item.name === "Edit"
      ? "수정"
      : item.name === "Write"
        ? item.resultKind === "update"
          ? "덮어쓰기"
          : item.resultKind === "create"
            ? "새 파일"
            : "파일 쓰기"
        : item.name === "NotebookEdit"
          ? "노트북 수정"
          : "여러 곳 수정",
  );

  // The applied diff. A Write that created a file reports no hunks, so its
  // whole content counts as added.
  const hunks = $derived.by<Hunk[] | null>(() => {
    if (!item.done || item.isError) return null;
    if (item.patch && item.patch.length > 0) return item.patch;
    if (item.name === "Write" && typeof item.input.content === "string") return [addedHunk(item.input.content)];
    return item.patch ?? null;
  });
  const counts = $derived(hunks ? countChanges(hunks) : null);

  /** The last `n` lines of a growing text, and how many lines were cut. */
  function tail(text: string, n: number): { text: string; cut: number } {
    const lines = text.split("\n");
    return lines.length <= n ? { text, cut: 0 } : { text: lines.slice(-n).join("\n"), cut: lines.length - n };
  }

  // Folded to one line (file, +/- counts, state): the files panel shows the
  // change in the file itself, and an approval card shows what it asks about.
  // Open by itself only for a failure, whose message is read here. A click
  // opens or folds it for good.
  let chosen = $state<boolean | null>(null);
  const open = $derived(chosen ?? phase === "error");
</script>

<div class="edit {phase}">
  <button class="head" onclick={() => (chosen = !open)} title="눌러서 접기/펼치기">
    <span class="icon">
      {#if phase === "writing"}✍{:else if phase === "awaiting"}⏸{:else if phase === "running"}<span class="spin"
        ></span>{:else if phase === "applied"}✓{:else}✗{/if}
    </span>
    <span class="label">{label}</span>
    <span class="path" title={path}>{path ? shortPath(path, base) : "파일 경로 작성 중…"}</span>
    {#if counts}
      <span class="counts"><span class="plus">+{counts.added}</span> <span class="minus">-{counts.removed}</span></span>
    {/if}
    <span class="phase">{PHASE_LABEL[phase]}</span>
  </button>
  {#if open}
    <div class="body">
      {#if hunks}
        {#if hunks.length > 0}
          <DiffHunks {hunks} max={300} />
        {:else}
          <div class="muted">바뀐 줄이 없습니다.</div>
        {/if}
      {:else if item.isError}
        <pre class="err">{clip(item.result ?? "", 4000)}</pre>
      {:else if item.name === "Write" || item.name === "NotebookEdit"}
        {@const t = tail(fields.content ?? fields.new_source ?? "", 40)}
        <pre class="add">{#if t.cut}<span class="cut">… 위 {t.cut}줄 생략
</span>{/if}{t.text}{#if phase === "writing"}<span class="caret">▍</span>{/if}</pre>
      {:else}
        {#if fields.old_string !== undefined}
          {@const t = tail(fields.old_string, 20)}
          <pre class="del">{#if t.cut}<span class="cut">… 위 {t.cut}줄 생략
</span>{/if}{t.text}</pre>
        {/if}
        {#if fields.new_string !== undefined}
          {@const t = tail(fields.new_string, 40)}
          <pre class="add">{#if t.cut}<span class="cut">… 위 {t.cut}줄 생략
</span>{/if}{t.text}{#if phase === "writing"}<span class="caret">▍</span>{/if}</pre>
        {:else if phase === "writing"}
          <div class="muted">바꿀 내용을 작성하는 중…</div>
        {/if}
      {/if}
    </div>
  {/if}
</div>

<style>
  .edit {
    border: 1px solid var(--border);
    border-left: 3px solid var(--info);
    border-radius: 6px;
    background: color-mix(in srgb, var(--surface-2) 60%, transparent);
    font-size: 0.9em;
  }
  .edit.writing {
    border-left-color: var(--accent);
    animation: breathe 1.4s ease-in-out infinite;
  }
  .edit.awaiting {
    border-color: var(--yellow);
  }
  .edit.applied {
    border-left-color: var(--green);
  }
  .edit.error {
    border-left-color: var(--red);
  }
  @keyframes breathe {
    50% {
      border-left-color: color-mix(in srgb, var(--accent) 35%, transparent);
    }
  }
  .head {
    display: flex;
    align-items: baseline;
    gap: 8px;
    width: 100%;
    text-align: left;
    font: inherit;
    color: var(--text);
    background: none;
    border: none;
    padding: 5px 8px;
    cursor: pointer;
  }
  .icon {
    flex: 0 0 auto;
    width: 1.1em;
    color: var(--green);
  }
  .edit.writing .icon {
    color: var(--accent);
  }
  .edit.error .icon {
    color: var(--red);
  }
  .label {
    flex: 0 0 auto;
    font-weight: 600;
  }
  .path {
    flex: 1 1 auto;
    min-width: 0;
    font-family: ui-monospace, "JetBrains Mono", "D2Coding", monospace;
    font-size: 0.92em;
    color: var(--text-2, var(--muted));
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .counts {
    flex: 0 0 auto;
    font-family: ui-monospace, "JetBrains Mono", "D2Coding", monospace;
    font-size: 0.9em;
  }
  .plus {
    color: var(--green);
  }
  .minus {
    color: var(--red);
  }
  .phase {
    flex: 0 0 auto;
    color: var(--muted);
    font-size: 0.85em;
  }
  .body {
    padding: 0 8px 8px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  pre {
    margin: 0;
    white-space: pre-wrap;
    word-break: break-all;
    font-family: ui-monospace, "JetBrains Mono", "D2Coding", monospace;
    font-size: 0.86em;
    line-height: 1.45;
    max-height: 420px;
    overflow: auto;
    border-radius: 4px;
    padding: 6px 8px;
  }
  pre.del {
    background: color-mix(in srgb, var(--red) 13%, var(--surface-2));
  }
  pre.add {
    background: color-mix(in srgb, var(--green) 13%, var(--surface-2));
  }
  pre.err {
    color: var(--red);
    background: var(--surface-2);
  }
  .cut {
    color: var(--muted);
  }
  .caret {
    color: var(--accent);
    animation: blink 1s steps(2) infinite;
  }
  @keyframes blink {
    50% {
      opacity: 0;
    }
  }
  .muted {
    color: var(--muted);
    font-size: 0.9em;
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
