<script lang="ts">
  // 지난 대화 검색 (Ctrl+Shift+H): find a past Claude conversation by what was
  // said in it, then read it in a read-only viewer scrolled to the match.
  // Resuming or forking a conversation is deliberately not offered — this is
  // for finding and re-reading. Keyboard-first, also fully clickable:
  //   list    type to search, ↑/↓ choose, Enter open, Esc close
  //   viewer  Enter / Shift+Enter next / previous match, Esc back to the list
  import { onMount, tick } from "svelte";
  import {
    historyConversation,
    searchHistory,
    type HistoryConversation,
    type HistoryHit,
  } from "./ipc";
  import { activePane, focusTerm, historySearch } from "./state.svelte";

  let query = $state("");
  let hits = $state<HistoryHit[]>([]);
  let selected = $state(0);
  // The first search reads every transcript once, which takes a moment.
  let loading = $state(true);
  let failed = $state<string | null>(null);
  let input = $state<HTMLInputElement>();
  let listEl = $state<HTMLDivElement>();

  let conversation = $state<HistoryConversation | null>(null);
  let viewTerms = $state<string[]>([]);
  // Messages (by index) that contain every term, and which of them is current.
  let matchAt = $state<number[]>([]);
  let current = $state(0);
  let viewerEl = $state<HTMLDivElement>();
  const matchSet = $derived(new Set(matchAt));

  const terms = $derived(query.trim().toLowerCase().split(/\s+/).filter(Boolean));

  // Answers can arrive out of order while typing; only the newest one counts.
  let latest = 0;
  async function run(q: string) {
    const mine = ++latest;
    try {
      const result = await searchHistory(q, 50);
      if (mine !== latest) return;
      hits = result;
      selected = 0;
      failed = null;
    } catch (e) {
      if (mine === latest) failed = String(e);
    } finally {
      if (mine === latest) loading = false;
    }
  }

  // Search as you type, a beat after the last keystroke (at once when empty).
  $effect(() => {
    const q = query.trim();
    const timer = setTimeout(() => void run(q), q ? 180 : 0);
    return () => clearTimeout(timer);
  });

  // Keep the highlighted row in view during keyboard navigation.
  $effect(() => {
    void selected;
    (listEl?.querySelector(".hs-item.sel") as HTMLElement | null)?.scrollIntoView({ block: "nearest" });
  });

  onMount(() => {
    input?.focus();
    // Whatever opened the search may move the keyboard once more (the palette
    // focuses the terminal right after running an action); claim it back.
    const refocus = setTimeout(() => input?.focus(), 0);
    return () => clearTimeout(refocus);
  });

  function close() {
    historySearch.open = false;
    void tick().then(() => focusTerm(activePane()));
  }

  async function openHit(hit: HistoryHit | undefined) {
    if (!hit) return;
    const conv = await historyConversation(hit.session).catch(() => null);
    if (!conv) return;
    const words = terms;
    viewTerms = words;
    matchAt = words.length
      ? conv.messages.flatMap((m, i) => {
          const text = m.text.toLowerCase();
          return words.every((w) => text.includes(w)) ? [i] : [];
        })
      : [];
    current = Math.max(0, matchAt.indexOf(hit.first_match ?? -1));
    conversation = conv;
    await tick();
    viewerEl?.focus();
    scrollToCurrent();
  }

  function backToList() {
    conversation = null;
    void tick().then(() => input?.focus());
  }

  function scrollToCurrent() {
    const at = matchAt[current];
    if (at === undefined) return;
    viewerEl?.querySelector(`[data-msg="${at}"]`)?.scrollIntoView({ block: "center" });
  }

  function step(offset: number) {
    if (matchAt.length === 0) return;
    current = (current + offset + matchAt.length) % matchAt.length;
    scrollToCurrent();
  }

  function onListKey(e: KeyboardEvent) {
    // Keep the search's own typing away from the global shortcut handlers.
    e.stopPropagation();
    if (e.isComposing || e.keyCode === 229) return;
    if (e.key === "Escape") {
      e.preventDefault();
      close();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      selected = Math.min(selected + 1, hits.length - 1);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      selected = Math.max(selected - 1, 0);
    } else if (e.key === "Enter") {
      e.preventDefault();
      void openHit(hits[selected]);
    } else if (e.ctrlKey && e.shiftKey && e.code === "KeyH") {
      e.preventDefault();
      close();
    }
  }

  function onViewerKey(e: KeyboardEvent) {
    // Arrow keys and Page Up/Down still scroll, and Ctrl+C still copies a
    // selection: only the keys below are taken.
    e.stopPropagation();
    if (e.key === "Escape") {
      e.preventDefault();
      backToList();
    } else if (e.key === "Enter") {
      e.preventDefault();
      step(e.shiftKey ? -1 : 1);
    } else if (e.ctrlKey && e.shiftKey && e.code === "KeyH") {
      e.preventDefault();
      close();
    }
  }

  function escapeRe(s: string): string {
    return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  }

  /** Split text into plain and matched runs, for highlighting. */
  function segments(text: string, words: string[]): { t: string; hit: boolean }[] {
    if (words.length === 0) return [{ t: text, hit: false }];
    const re = new RegExp(words.map(escapeRe).join("|"), "gi");
    const out: { t: string; hit: boolean }[] = [];
    let last = 0;
    for (const m of text.matchAll(re)) {
      const at = m.index ?? 0;
      if (at > last) out.push({ t: text.slice(last, at), hit: false });
      out.push({ t: m[0], hit: true });
      last = at + m[0].length;
    }
    if (last < text.length) out.push({ t: text.slice(last), hit: false });
    return out;
  }

  function shortPath(cwd: string | null): string {
    if (!cwd) return "";
    let s = cwd;
    if (s.startsWith("/home/")) {
      const i = s.indexOf("/", 6);
      s = i > 0 ? "~" + s.slice(i) : "~";
    }
    const parts = s.split("/");
    return parts.length > 4 ? "…/" + parts.slice(-2).join("/") : s;
  }

  function when(iso: string | null): string {
    if (!iso) return "";
    const d = new Date(iso);
    if (Number.isNaN(d.getTime())) return "";
    const pad = (n: number) => String(n).padStart(2, "0");
    const day = `${d.getMonth() + 1}/${d.getDate()} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
    return d.getFullYear() === new Date().getFullYear() ? day : `${d.getFullYear()}/${day}`;
  }
</script>

<div class="hs-backdrop" onpointerdown={close} role="presentation">
  <div
    class="hs"
    role="dialog"
    aria-modal="true"
    tabindex="-1"
    onpointerdown={(e) => e.stopPropagation()}
    onkeydown={(e) => {
      // Whatever has focus inside the dialog, its keys stay here and never
      // reach the shortcuts behind it. The list's input handles (and stops)
      // its own keys first, so in list view this only sees keys from
      // elsewhere in the dialog, such as a result row that was clicked.
      if (conversation) onViewerKey(e);
      else onListKey(e);
    }}
  >
    {#if conversation}
      <div class="hv-head">
        <button class="hv-back" onclick={backToList} title="목록으로 (Esc)">← 목록</button>
        <div class="hv-title">
          <span class="hv-name">{conversation.title ?? "(제목 없음)"}</span>
          <span class="hv-meta">
            {shortPath(conversation.cwd)}{conversation.branch ? `  ⎇ ${conversation.branch}` : ""}
          </span>
        </div>
        {#if matchAt.length > 0}
          <div class="hv-nav">
            <button onclick={() => step(-1)} title="이전 (Shift+Enter)">▲</button>
            <span>{current + 1} / {matchAt.length}</span>
            <button onclick={() => step(1)} title="다음 (Enter)">▼</button>
          </div>
        {/if}
      </div>
      <div class="hv-body" bind:this={viewerEl} tabindex="-1">
        {#each conversation.messages as m, i (i)}
          <div
            class="hv-msg {m.role}"
            class:match={matchSet.has(i)}
            class:current={matchAt[current] === i}
            data-msg={i}
          >
            <div class="hv-who">
              {m.role === "user" ? "나" : "Claude"}
              <span class="hv-time">{when(m.ts)}</span>
            </div>
            <!-- Kept on one line: the text is pre-wrap, so template whitespace
                 inside this div would show up as a leading blank line. -->
            <div class="hv-text">{#each segments(m.text, viewTerms) as s, k (k)}{#if s.hit}<mark>{s.t}</mark>{:else}{s.t}{/if}{/each}</div>
          </div>
        {/each}
      </div>
    {:else}
      <input
        class="hs-input"
        bind:this={input}
        bind:value={query}
        onkeydown={onListKey}
        placeholder="지난 대화 검색…   (↑↓ 이동 / Enter 열기 / Esc 닫기)"
        spellcheck="false"
        autocomplete="off"
      />
      <div class="hs-list" bind:this={listEl}>
        {#each hits as hit, i (hit.session)}
          <button
            class="hs-item"
            class:sel={i === selected}
            onmousemove={() => (selected = i)}
            onclick={() => void openHit(hit)}
          >
            <div class="hs-row">
              <span class="hs-name">{hit.title ?? "(제목 없음)"}</span>
              <span class="hs-meta">{shortPath(hit.cwd)}</span>
              <span class="hs-when">{when(hit.last_ts)}</span>
              {#if hit.matches > 1}<span class="hs-count">{hit.matches}곳</span>{/if}
            </div>
            {#if hit.snippet}
              <div class="hs-snippet">
                {#each segments(hit.snippet, terms) as s, k (k)}{#if s.hit}<mark>{s.t}</mark>{:else}{s.t}{/if}{/each}
              </div>
            {/if}
          </button>
        {/each}
        {#if failed}
          <div class="hs-empty">대화 기록을 읽지 못했습니다: {failed}</div>
        {:else if loading && hits.length === 0}
          <div class="hs-empty">대화 기록을 읽는 중…</div>
        {:else if hits.length === 0}
          <div class="hs-empty">{terms.length ? "일치하는 대화가 없습니다" : "지난 대화가 없습니다"}</div>
        {/if}
      </div>
    {/if}
  </div>
</div>

<style>
  .hs-backdrop {
    position: fixed;
    inset: 0;
    z-index: 10000;
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding-top: 8vh;
    background: color-mix(in srgb, #000 45%, transparent);
  }
  .hs {
    width: min(860px, 94vw);
    height: 78vh;
    display: flex;
    flex-direction: column;
    background: var(--surface-2);
    border: 1px solid var(--border-2);
    border-radius: 12px;
    box-shadow: 0 16px 48px rgba(0, 0, 0, 0.55);
    overflow: hidden;
  }
  .hs-input {
    width: 100%;
    padding: 14px 16px;
    font-size: 1rem;
    color: var(--text);
    background: transparent;
    border: none;
    border-bottom: 1px solid var(--border);
    outline: none;
  }
  .hs-list {
    flex: 1;
    overflow-y: auto;
    padding: 6px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .hs-item {
    display: flex;
    flex-direction: column;
    gap: 3px;
    width: 100%;
    padding: 8px 10px;
    text-align: left;
    color: var(--text);
    background: none;
    border: none;
    border-radius: 7px;
    cursor: pointer;
  }
  .hs-item.sel {
    background: color-mix(in srgb, var(--accent) 22%, transparent);
  }
  .hs-row {
    display: flex;
    align-items: baseline;
    gap: 10px;
    min-width: 0;
  }
  .hs-name {
    flex-shrink: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 0.9rem;
    font-weight: 600;
  }
  .hs-meta,
  .hs-when,
  .hs-count {
    flex-shrink: 0;
    font-size: 0.72rem;
    color: var(--muted);
    white-space: nowrap;
  }
  .hs-when {
    margin-left: auto;
  }
  .hs-count {
    padding: 0 6px;
    border-radius: 6px;
    background: color-mix(in srgb, var(--accent) 18%, transparent);
  }
  .hs-snippet {
    font-size: 0.78rem;
    color: var(--muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .hs-empty {
    padding: 18px;
    text-align: center;
    color: var(--muted);
    font-size: 0.85rem;
  }
  mark {
    color: var(--bg);
    background: var(--yellow);
    border-radius: 2px;
  }

  /* Read-only viewer. */
  .hv-head {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 14px;
    border-bottom: 1px solid var(--border);
  }
  .hv-back,
  .hv-nav button {
    flex-shrink: 0;
    padding: 4px 10px;
    font: inherit;
    font-size: 0.8rem;
    color: var(--muted);
    background: transparent;
    border: 1px solid var(--border);
    border-radius: 6px;
    cursor: pointer;
  }
  .hv-back:hover,
  .hv-nav button:hover {
    color: var(--text);
    background: color-mix(in srgb, var(--text) 10%, transparent);
  }
  .hv-title {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .hv-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 600;
    color: var(--text);
  }
  .hv-meta {
    font-size: 0.72rem;
    color: var(--muted);
  }
  .hv-nav {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 0.78rem;
    color: var(--muted);
  }
  .hv-body {
    flex: 1;
    overflow-y: auto;
    padding: 12px 16px 24px;
    outline: none;
  }
  .hv-msg {
    margin-bottom: 12px;
    padding: 8px 12px;
    border-radius: 8px;
    border: 1px solid transparent;
  }
  .hv-msg.user {
    background: color-mix(in srgb, var(--accent) 9%, transparent);
  }
  .hv-msg.match {
    border-color: color-mix(in srgb, var(--yellow) 45%, transparent);
  }
  .hv-msg.current {
    border-color: var(--yellow);
    box-shadow: 0 0 0 1px var(--yellow);
  }
  .hv-who {
    margin-bottom: 4px;
    font-size: 0.72rem;
    font-weight: 700;
    color: var(--accent);
  }
  .hv-msg.assistant .hv-who {
    color: var(--info);
  }
  .hv-time {
    margin-left: 6px;
    font-weight: 400;
    color: var(--muted);
  }
  /* The body is user-select:none app-wide; conversation text must be copyable. */
  .hv-text {
    font-size: 0.84rem;
    line-height: 1.6;
    color: var(--text);
    white-space: pre-wrap;
    word-break: break-word;
    user-select: text;
    cursor: text;
  }
</style>
