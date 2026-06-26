<script lang="ts">
  // Command Palette (Ctrl+Shift+P): one fuzzy search over every action, pane,
  // workspace and theme. Keyboard-first (↑/↓/Enter/Esc), also fully clickable.
  import { tick } from "svelte";
  import { closePane, createWorkspace, focusPane, focusWorkspace, splitPane } from "./ipc";
  import { app, activePane, broadcast, palette, focusTerm } from "./state.svelte";
  import { settings, setTheme } from "./settings.svelte";
  import { THEMES } from "./themes";

  type Kind = "action" | "workspace" | "pane" | "theme";
  interface Item {
    kind: Kind;
    label: string;
    detail?: string;
    hint?: string;
    status?: string;
    run: () => void;
  }

  let query = $state("");
  let selected = $state(0);
  let input = $state<HTMLInputElement>();
  let listEl = $state<HTMLDivElement>();

  function kindLabel(k: Kind): string {
    return k === "action" ? "동작" : k === "workspace" ? "WS" : k === "pane" ? "PANE" : "테마";
  }

  function shortCwd(cwd: string | null): string {
    if (!cwd) return "";
    let s = cwd;
    if (s.startsWith("/home/")) {
      const i = s.indexOf("/", 6);
      s = i > 0 ? "~" + s.slice(i) : "~";
    }
    const parts = s.split("/");
    return parts.length > 4 ? "…/" + parts.slice(-2).join("/") : s;
  }

  // Every candidate the palette can act on — rebuilt reactively from the
  // snapshot so pane names, statuses and the current theme stay live.
  const items = $derived.by<Item[]>(() => {
    const snap = app.snapshot;
    const out: Item[] = [];
    const ap = activePane();
    out.push({
      kind: "action",
      label: "⚡ Broadcast 모드 토글",
      hint: broadcast.on ? "켜짐" : "꺼짐",
      run: () => (broadcast.on = !broadcast.on),
    });
    if (ap) {
      out.push({ kind: "action", label: "오른쪽으로 분할", run: () => void splitPane(ap, "horizontal") });
      out.push({ kind: "action", label: "아래로 분할", run: () => void splitPane(ap, "vertical") });
      out.push({ kind: "action", label: "Pane 닫기", run: () => void closePane(ap) });
    }
    out.push({ kind: "action", label: "새 워크스페이스", run: () => void createWorkspace() });
    for (const ws of snap?.workspaces ?? []) {
      out.push({ kind: "workspace", label: ws.name, detail: "워크스페이스 전환", run: () => void focusWorkspace(ws.id) });
    }
    for (const p of snap?.panes ?? []) {
      if (p.exited) continue;
      const wsName = snap?.workspaces.find((w) => w.id === p.workspace)?.name ?? "";
      const detail = [p.meta.git_branch ? "⎇ " + p.meta.git_branch : null, shortCwd(p.meta.cwd), wsName]
        .filter(Boolean)
        .join("  ·  ");
      out.push({ kind: "pane", label: p.name, detail, status: p.status, run: () => void focusPane(p.id) });
    }
    for (const t of THEMES) {
      out.push({
        kind: "theme",
        label: "테마: " + t.name,
        hint: t.id === settings.theme ? "현재" : "",
        run: () => setTheme(t.id),
      });
    }
    return out;
  });

  // Subsequence fuzzy match: streak + word-start bonuses, slight shorter-better.
  function fuzzyScore(q: string, text: string): number {
    q = q.toLowerCase();
    const t = text.toLowerCase();
    let qi = 0,
      score = 0,
      last = -2,
      streak = 0;
    for (let ti = 0; ti < t.length && qi < q.length; ti++) {
      if (t[ti] === q[qi]) {
        score += 10;
        if (last === ti - 1) {
          streak++;
          score += streak * 5;
        } else streak = 0;
        if (ti === 0 || t[ti - 1] === " ") score += 8;
        last = ti;
        qi++;
      }
    }
    return qi === q.length ? score - t.length * 0.1 : -1;
  }

  const filtered = $derived.by<Item[]>(() => {
    const q = query.trim();
    if (!q) return items.slice(0, 60);
    const scored: { it: Item; s: number }[] = [];
    for (const it of items) {
      const s = fuzzyScore(q, it.label + " " + (it.detail ?? ""));
      if (s >= 0) scored.push({ it, s });
    }
    scored.sort((a, b) => b.s - a.s);
    return scored.slice(0, 60).map((x) => x.it);
  });

  // Reset highlight whenever the query changes.
  $effect(() => {
    void query;
    selected = 0;
  });

  // Keep the highlighted row in view during keyboard navigation.
  $effect(() => {
    void selected;
    (listEl?.querySelector(".pal-item.sel") as HTMLElement | null)?.scrollIntoView({ block: "nearest" });
  });

  // Focus the input as soon as it mounts.
  $effect(() => {
    input?.focus();
  });

  function close() {
    palette.open = false;
    query = "";
    void tick().then(() => focusTerm(activePane()));
  }

  function exec(it: Item | undefined) {
    palette.open = false;
    query = "";
    it?.run();
    void tick().then(() => focusTerm(activePane()));
  }

  function onkeydown(e: KeyboardEvent) {
    // Keep the palette's own typing away from the global shortcut handlers.
    e.stopPropagation();
    if (e.key === "Escape") {
      e.preventDefault();
      close();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      selected = Math.min(selected + 1, filtered.length - 1);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      selected = Math.max(selected - 1, 0);
    } else if (e.key === "Enter") {
      e.preventDefault();
      exec(filtered[Math.min(selected, filtered.length - 1)]);
    } else if (e.ctrlKey && e.shiftKey && e.code === "KeyP") {
      e.preventDefault();
      close();
    }
  }
</script>

<div class="pal-backdrop" onpointerdown={close} role="presentation">
  <div class="pal" role="dialog" aria-modal="true" tabindex="-1" onpointerdown={(e) => e.stopPropagation()}>
    <input
      class="pal-input"
      bind:this={input}
      bind:value={query}
      {onkeydown}
      placeholder="명령 · pane · 워크스페이스 · 테마 검색…   (↑↓ 이동 · Enter 실행 · Esc 닫기)"
      spellcheck="false"
      autocomplete="off"
    />
    <div class="pal-list" bind:this={listEl}>
      {#each filtered as it, i (it.kind + "·" + it.label + "·" + i)}
        <button
          class="pal-item"
          class:sel={i === selected}
          onmousemove={() => (selected = i)}
          onclick={() => exec(it)}
        >
          <span class="pal-kind {it.kind}">{kindLabel(it.kind)}</span>
          {#if it.kind === "pane" && it.status}
            <span class="pal-dot {it.status}"></span>
          {/if}
          <span class="pal-label">{it.label}</span>
          {#if it.detail}<span class="pal-detail">{it.detail}</span>{/if}
          {#if it.hint}<span class="pal-hint">{it.hint}</span>{/if}
        </button>
      {/each}
      {#if filtered.length === 0}
        <div class="pal-empty">일치하는 항목이 없습니다</div>
      {/if}
    </div>
  </div>
</div>

<style>
  .pal-backdrop {
    position: fixed;
    inset: 0;
    z-index: 10000;
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding-top: 12vh;
    background: color-mix(in srgb, #000 45%, transparent);
  }
  .pal {
    width: min(640px, 92vw);
    max-height: 70vh;
    display: flex;
    flex-direction: column;
    background: var(--surface-2);
    border: 1px solid var(--border-2);
    border-radius: 12px;
    box-shadow: 0 16px 48px rgba(0, 0, 0, 0.55);
    overflow: hidden;
  }
  .pal-input {
    width: 100%;
    padding: 14px 16px;
    font-size: 1rem;
    color: var(--text);
    background: transparent;
    border: none;
    border-bottom: 1px solid var(--border);
    outline: none;
  }
  .pal-list {
    overflow-y: auto;
    padding: 6px;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .pal-item {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 8px 10px;
    font-size: 0.9rem;
    text-align: left;
    color: var(--text);
    background: none;
    border: none;
    border-radius: 7px;
    cursor: pointer;
  }
  .pal-item.sel {
    background: color-mix(in srgb, var(--accent) 22%, transparent);
  }
  .pal-kind {
    flex-shrink: 0;
    min-width: 38px;
    text-align: center;
    font-size: 0.6rem;
    font-weight: 700;
    letter-spacing: 0.03em;
    padding: 2px 6px;
    border-radius: 6px;
    color: var(--bg);
    background: var(--muted);
  }
  .pal-kind.action {
    background: var(--accent);
  }
  .pal-kind.workspace {
    background: var(--info);
  }
  .pal-kind.pane {
    background: var(--green);
  }
  .pal-dot {
    flex-shrink: 0;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--muted);
  }
  .pal-dot.processing {
    background: var(--red);
  }
  .pal-dot.processed {
    background: var(--green);
  }
  .pal-dot.idle {
    background: var(--accent);
  }
  .pal-dot.waiting {
    background: var(--yellow);
  }
  .pal-label {
    flex-shrink: 0;
    white-space: nowrap;
  }
  .pal-detail {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--muted);
    font-size: 0.78rem;
  }
  .pal-hint {
    margin-left: auto;
    flex-shrink: 0;
    color: var(--muted);
    font-size: 0.72rem;
  }
  .pal-empty {
    padding: 18px;
    text-align: center;
    color: var(--muted);
    font-size: 0.85rem;
  }
</style>
