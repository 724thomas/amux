<script lang="ts">
  // Who-Needs-Me: a prioritized rail of the agents that want you right now —
  // 🟡 waiting (needs input) above 🟢 processed (done, unseen), oldest first,
  // each with a live timer. Click a row to jump straight to that pane.
  import { focusPane } from "./ipc";
  import { attentionItems, clock } from "./state.svelte";

  const items = $derived(attentionItems());

  function fmt(since: number): string {
    const s = Math.max(0, Math.floor((clock.now - since) / 1000));
    return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
  }
</script>

{#if items.length > 0}
  <div class="rail">
    <div class="rail-head">
      <span class="warn">⚠</span>
      지금 봐야 할 에이전트
      <span class="count">{items.length}</span>
    </div>
    {#each items as it (it.pane)}
      <button class="row" onclick={() => void focusPane(it.pane).catch(() => {})}>
        <span class="dot {it.status}"></span>
        <span class="nm">{it.name}</span>
        {#if it.workspace}<span class="ws">{it.workspace}</span>{/if}
        <span class="lbl">{it.status === "waiting" ? "입력 대기" : "완료·미확인"}</span>
        <span class="time">{fmt(it.since)}</span>
      </button>
    {/each}
  </div>
{/if}

<style>
  .rail {
    margin: 8px 8px 4px;
    border: 1px solid color-mix(in srgb, var(--yellow) 40%, var(--border-2));
    border-radius: 8px;
    background: color-mix(in srgb, var(--yellow) 7%, var(--surface-2));
    overflow: hidden;
  }
  .rail-head {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    font-size: 0.7rem;
    font-weight: 700;
    color: var(--text-2);
    border-bottom: 1px solid var(--border);
  }
  .rail-head .warn {
    color: var(--yellow);
  }
  .rail-head .count {
    margin-left: auto;
    min-width: 16px;
    padding: 0 6px;
    text-align: center;
    border-radius: 9px;
    background: var(--yellow);
    color: var(--bg);
  }
  .row {
    display: flex;
    align-items: center;
    gap: 7px;
    width: 100%;
    padding: 6px 10px;
    background: none;
    border: none;
    border-top: 1px solid color-mix(in srgb, var(--border) 55%, transparent);
    color: var(--text);
    cursor: pointer;
    font-size: 0.74rem;
    text-align: left;
  }
  .row:first-of-type {
    border-top: none;
  }
  .row:hover {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  .dot {
    flex-shrink: 0;
    width: 7px;
    height: 7px;
    border-radius: 50%;
  }
  .dot.waiting {
    background: var(--yellow);
    animation: rail-pulse 1.2s ease-in-out infinite;
  }
  .dot.processed {
    background: var(--green);
  }
  @keyframes rail-pulse {
    0%,
    100% {
      box-shadow: 0 0 0 0 transparent;
    }
    50% {
      box-shadow: 0 0 7px 0 var(--yellow);
    }
  }
  .nm {
    flex-shrink: 0;
    max-width: 42%;
    overflow: hidden;
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ws {
    flex-shrink: 0;
    color: var(--muted);
    font-size: 0.68rem;
  }
  .lbl {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    color: var(--muted);
    text-align: right;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .time {
    flex-shrink: 0;
    min-width: 30px;
    text-align: right;
    color: var(--text-2);
    font-variant-numeric: tabular-nums;
  }
</style>
