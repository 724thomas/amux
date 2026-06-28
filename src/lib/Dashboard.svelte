<script lang="ts">
  // Mission Control — a JARVIS / Iron-Man HUD board of every live agent across
  // all workspaces. Each agent is a glowing "reactor" node: a status-colored
  // ring with a rotating arc sweep + a centred readout. Rendered in the
  // dashboard overlay (Ctrl+Shift+A). A node click jumps to that pane.
  import { focusPane, focusWorkspace } from "./ipc";
  import { dashboardAgents, statusCounts, clock, type AgentTile } from "./state.svelte";

  let { dismiss }: { dismiss?: () => void } = $props();

  const agents = $derived(dashboardAgents());
  const counts = $derived(statusCounts());

  function ago(since: number): string {
    const s = Math.max(0, Math.floor((clock.now - since) / 1000));
    if (s < 60) return `${s}s`;
    if (s < 3600) return `${Math.floor(s / 60)}m`;
    return `${Math.floor(s / 3600)}h`;
  }
  async function jump(t: AgentTile) {
    dismiss?.();
    await focusWorkspace(t.workspaceId);
    await focusPane(t.pane);
  }
</script>

<div class="dash">
  <header class="head">
    <span class="title"><span class="reticle"></span>MISSION&nbsp;CONTROL</span>
    <span class="agg">
      <span class="c proc"><i></i>{counts.processing} ACTIVE</span>
      <span class="c wait"><i></i>{counts.waiting} WAIT</span>
      <span class="c done"><i></i>{counts.processed} DONE</span>
      <span class="c idle"><i></i>{counts.idle} IDLE</span>
      <span class="c total">{counts.total} UNITS ONLINE</span>
    </span>
  </header>

  <div class="grid">
    {#each agents as a (a.pane)}
      <button class="node" data-status={a.status} onclick={() => jump(a)} title="{a.name} — {a.status}">
        <span class="ring">
          <span class="pct">{ago(a.since)}</span>
        </span>
        <span class="name">{a.name}</span>
        <span class="meta">{a.status}{#if a.branch} · ⎇{a.branch}{/if}</span>
        <span class="ws">{a.workspace}</span>
      </button>
    {/each}
    {#if agents.length === 0}
      <div class="empty">활성 에이전트가 없습니다</div>
    {/if}
  </div>
</div>

<style>
  .dash {
    position: relative;
    z-index: 1; /* above the modal's grid (::before), below its scan/brackets */
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100%;
    min-height: 0;
    background: transparent;
    color: var(--text);
    font-family: ui-monospace, "SF Mono", monospace;
  }

  /* ── Header ── */
  .head {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 10px 18px;
    padding: 16px 24px;
  }
  .head::after {
    content: "";
    position: absolute;
    left: 24px;
    right: 24px;
    bottom: 0;
    height: 1px;
    background: linear-gradient(
      90deg,
      transparent,
      color-mix(in srgb, var(--info) 80%, transparent),
      transparent
    );
    box-shadow: 0 0 8px color-mix(in srgb, var(--info) 55%, transparent);
  }
  .title {
    display: flex;
    align-items: center;
    gap: 11px;
    font-weight: 700;
    font-size: 0.98rem;
    letter-spacing: 0.28em;
    color: var(--info);
    text-shadow: 0 0 12px color-mix(in srgb, var(--info) 75%, transparent);
  }
  .reticle {
    width: 11px;
    height: 11px;
    border: 1.5px solid var(--info);
    border-radius: 2px;
    box-shadow:
      0 0 8px var(--info),
      inset 0 0 4px var(--info);
    animation: ret 2.4s ease-in-out infinite;
  }
  @keyframes ret {
    0%,
    100% {
      opacity: 1;
      transform: rotate(0);
    }
    50% {
      opacity: 0.4;
      transform: rotate(45deg);
    }
  }
  .agg {
    display: flex;
    align-items: center;
    gap: 14px;
    font-size: 0.7rem;
    letter-spacing: 0.13em;
  }
  .agg .c {
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .agg .c i {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: currentColor;
    box-shadow: 0 0 7px currentColor;
  }
  .agg .proc {
    color: var(--red);
  }
  .agg .wait {
    color: var(--yellow);
  }
  .agg .done {
    color: var(--green);
  }
  .agg .idle {
    color: var(--accent);
  }
  .agg .total {
    color: var(--info);
    opacity: 0.7;
    letter-spacing: 0.15em;
  }

  /* ── Reactor-node grid ── */
  .grid {
    flex: 1;
    min-height: 0;
    overflow: auto;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(152px, 1fr));
    gap: 22px 12px;
    padding: 28px 22px;
    align-content: start;
    justify-items: center;
  }
  .node {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 4px;
    background: none;
    border: none;
    cursor: pointer;
    color: var(--text);
    transition: transform 0.14s ease;
    --st: var(--accent);
  }
  .node[data-status="processing"] {
    --st: var(--red);
  }
  .node[data-status="waiting"] {
    --st: var(--yellow);
  }
  .node[data-status="processed"] {
    --st: var(--green);
  }
  .node[data-status="idle"] {
    --st: var(--accent);
  }
  .node:hover {
    transform: translateY(-3px) scale(1.05);
  }

  .ring {
    position: relative;
    display: grid;
    place-items: center;
    width: 82px;
    height: 82px;
    border-radius: 50%;
    border: 2px solid color-mix(in srgb, var(--st) 28%, transparent);
    box-shadow:
      0 0 16px -2px color-mix(in srgb, var(--st) 55%, transparent),
      inset 0 0 16px -5px color-mix(in srgb, var(--st) 45%, transparent);
  }
  /* rotating reactor sweep on the rim */
  .ring::before {
    content: "";
    position: absolute;
    inset: -2px;
    border-radius: 50%;
    background: conic-gradient(from 0deg, transparent 0 62%, var(--st) 88%, transparent 100%);
    -webkit-mask: radial-gradient(farthest-side, transparent calc(100% - 4px), #000 calc(100% - 4px));
    mask: radial-gradient(farthest-side, transparent calc(100% - 4px), #000 calc(100% - 4px));
    filter: drop-shadow(0 0 4px var(--st));
    animation: spin 3s linear infinite;
  }
  /* inner dashed tick ring */
  .ring::after {
    content: "";
    position: absolute;
    inset: 10px;
    border-radius: 50%;
    border: 1px dashed color-mix(in srgb, var(--st) 24%, transparent);
  }
  @keyframes spin {
    to {
      transform: rotate(1turn);
    }
  }
  .node[data-status="processing"] .ring::before {
    animation-duration: 1.4s;
  }
  .node[data-status="processing"] .ring {
    animation: pulse 1.7s ease-in-out infinite;
  }
  .node[data-status="waiting"] .ring::before {
    animation-duration: 2.4s;
  }
  /* done = a near-complete bright ring, slow drift */
  .node[data-status="processed"] .ring::before {
    background: conic-gradient(from 0deg, var(--st) 0 90%, transparent 100%);
    animation-duration: 7s;
    opacity: 0.7;
  }
  .node[data-status="idle"] .ring::before {
    animation: none;
    opacity: 0.18;
  }
  .node[data-status="idle"] .ring {
    opacity: 0.7;
  }
  @keyframes pulse {
    0%,
    100% {
      box-shadow:
        0 0 16px -2px color-mix(in srgb, var(--st) 55%, transparent),
        inset 0 0 16px -5px color-mix(in srgb, var(--st) 45%, transparent);
    }
    50% {
      box-shadow:
        0 0 26px 0 color-mix(in srgb, var(--st) 78%, transparent),
        inset 0 0 20px -3px color-mix(in srgb, var(--st) 62%, transparent);
    }
  }
  .pct {
    font: 700 0.84rem/1 ui-monospace, monospace;
    letter-spacing: 0.03em;
    color: var(--st);
    text-shadow: 0 0 9px color-mix(in srgb, var(--st) 75%, transparent);
  }

  .name {
    max-width: 100%;
    font-weight: 600;
    font-size: 0.8rem;
    letter-spacing: 0.03em;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    text-shadow: 0 0 6px color-mix(in srgb, var(--info) 30%, transparent);
  }
  .meta {
    max-width: 100%;
    font-size: 0.62rem;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    opacity: 0.7;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ws {
    font-size: 0.58rem;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--info);
    opacity: 0.55;
  }
  .empty {
    grid-column: 1 / -1;
    padding: 48px;
    text-align: center;
    letter-spacing: 0.1em;
    opacity: 0.5;
  }

  @media (prefers-reduced-motion: reduce) {
    .reticle,
    .ring,
    .ring::before {
      animation: none !important;
    }
  }
</style>
