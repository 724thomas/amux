<script lang="ts">
  // Diff hunks as rows with old and new line numbers.
  import { hunkRows, type Hunk } from "./diff";

  let { hunks, max = 400 }: { hunks: Hunk[]; max?: number } = $props();

  const rows = $derived(hunkRows(hunks));
  let showAll = $state(false);
  const shown = $derived(showAll ? rows : rows.slice(0, max));
</script>

<div class="diff">
  {#each shown as r, i (i)}
    {#if r.kind === "gap"}
      <div class="gap">⋯</div>
    {:else}
      <div class="row {r.kind}">
        <span class="no">{r.old ?? ""}</span>
        <span class="no">{r.new ?? ""}</span>
        <span class="sign">{r.kind === "add" ? "+" : r.kind === "del" ? "-" : ""}</span>
        <span class="text">{r.text}</span>
      </div>
    {/if}
  {/each}
  {#if rows.length > shown.length}
    <button class="more" onclick={() => (showAll = true)}>나머지 {rows.length - shown.length}줄 더 보기</button>
  {/if}
</div>

<style>
  .diff {
    font-family: ui-monospace, "JetBrains Mono", "D2Coding", monospace;
    font-size: 0.84em;
    line-height: 1.45;
    background: var(--surface-2);
    border-radius: 4px;
    overflow-x: auto;
  }
  .row {
    display: grid;
    grid-template-columns: 3.4em 3.4em 1.1em minmax(0, 1fr);
    min-width: 0;
  }
  .row.add {
    background: color-mix(in srgb, var(--green) 15%, transparent);
  }
  .row.del {
    background: color-mix(in srgb, var(--red) 15%, transparent);
  }
  .no {
    color: var(--muted);
    text-align: right;
    padding-right: 6px;
    user-select: none;
    opacity: 0.75;
  }
  .sign {
    user-select: none;
    color: var(--muted);
  }
  .row.add .sign {
    color: var(--green);
  }
  .row.del .sign {
    color: var(--red);
  }
  .text {
    white-space: pre-wrap;
    word-break: break-all;
    padding-right: 8px;
  }
  .gap {
    color: var(--muted);
    text-align: center;
    font-size: 0.85em;
    border-top: 1px dashed var(--border);
    border-bottom: 1px dashed var(--border);
  }
  .more {
    display: block;
    width: 100%;
    font: inherit;
    color: var(--info);
    background: none;
    border: none;
    padding: 4px;
    cursor: pointer;
  }
</style>
