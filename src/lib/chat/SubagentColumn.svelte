<script lang="ts">
  // The column on the right of a pane (2 of 3:2) with one card per subagent of
  // the Claude on screen in it, stacked top to bottom. Always mounted, so its
  // look loop keeps running; it takes no room while it has no cards.
  import { onDestroy, untrack } from "svelte";
  import type { PaneId } from "../ipc";
  import { paneInfo, paneOnScreen } from "../state.svelte";
  import { settings } from "../settings.svelte";
  import { chatOf } from "./chat.svelte";
  import { useLinks, subLinks } from "./subagentLinks.svelte";
  import { closeCard, forgetPane, poll, subs, type Source } from "./subagentColumn.svelte";
  import SubagentCard from "./SubagentCard.svelte";

  let { pane, chat }: { pane: PaneId; chat: boolean } = $props();

  /** The chat's Claude while the chat is on screen, else the terminal's. */
  const src = $derived.by((): Source | null => {
    if (chat) {
      const s = chatOf(pane);
      const session = s?.model.sessionId;
      if (!s || !session) return null;
      return {
        session,
        alive: s.gen != null,
        startedMs: s.startedMs,
        prompts: s.model.userSeq,
        busy: s.model.status === "working" || s.model.status === "waiting",
      };
    }
    const l = subLinks.byPane[pane];
    return l ? { session: l.session, alive: true, startedMs: l.started_ms, prompts: null, busy: l.busy } : null;
  });
  /** What makes a look worth taking at once (not the busy flag, which only
   *  changes the pace). */
  const kickKey = $derived(src ? `${src.session}|${src.alive}|${src.prompts}` : "");

  const agents = $derived(subs.byPane[pane]?.agents ?? []);
  const cwd = $derived(
    (chat ? chatOf(pane)?.model.cwd : null) ?? paneInfo(pane)?.meta.cwd ?? null,
  );

  let timer: ReturnType<typeof setTimeout> | null = null;
  let inFlight = false;
  let again = false;
  let gone = false;

  async function look() {
    timer = null;
    if (inFlight) {
      again = true;
      return;
    }
    inFlight = true;
    let soon = false;
    try {
      soon = await poll(pane, untrack(() => src));
    } catch {
      // An older backend without these commands, or a transient read error.
    }
    inFlight = false;
    if (gone) return;
    if (again) {
      again = false;
      timer = setTimeout(look, 0);
      return;
    }
    const shown = paneOnScreen(pane) && !document.hidden;
    timer = setTimeout(look, !shown ? 5000 : soon ? 1000 : 2500);
  }
  function kick() {
    if (timer) clearTimeout(timer);
    void look();
  }

  const release = useLinks();
  $effect(() => {
    void kickKey;
    untrack(kick);
  });
  onDestroy(() => {
    gone = true;
    if (timer) clearTimeout(timer);
    release();
    // A closed pane: nothing will come back for its cards. (A pane view that
    // is only being rebuilt still has its pane in the snapshot.)
    if (!paneInfo(pane)) forgetPane(pane);
  });
</script>

{#if agents.length}
  <aside class="subcol" style="--sub-fs: {settings.fontSize}px" aria-label="서브에이전트">
    {#each agents as a (a.id)}
      <SubagentCard agent={a} {cwd} onclose={() => closeCard(pane, a.id)} />
    {/each}
  </aside>
{/if}

<style>
  /* 2 of the pane's 3:2 split; the main part beside it takes the rest. */
  .subcol {
    flex: 0 0 40%;
    min-width: 0;
    display: flex;
    flex-direction: column;
    border-left: 1px solid var(--border);
    background: var(--bg);
    overflow-y: auto;
    font-family: system-ui, sans-serif;
    font-size: var(--sub-fs, 14px);
  }
</style>
