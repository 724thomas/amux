<script lang="ts">
  // The files panel beside a terminal (CLI mode). A terminal shows amux only
  // text, not Claude's tool calls, so nothing is typed while Claude writes it.
  // Instead every change is played back from disk right after it lands, the
  // same way the chat plays back changes made by commands. When to look comes
  // from the pane's status (see statusFeed.ts).
  import { onDestroy, untrack } from "svelte";
  import FilesPanel from "./FilesPanel.svelte";
  import { emptyFeed, statusFeed } from "./statusFeed";
  import { popOut } from "./popout.svelte";
  import type { PaneId } from "../ipc";
  import { focusTerm, paneInfo, paneOnScreen } from "../state.svelte";

  let { pane, overlay = false, onclose }: { pane: PaneId; overlay?: boolean; onclose: () => void } = $props();

  const cwd = $derived(paneInfo(pane)?.meta.cwd ?? null);
  const onScreen = $derived(paneOnScreen(pane));

  const feed = $state(emptyFeed());
  const driver = statusFeed(feed);
  onDestroy(() => driver.dispose());
  // Runs on every snapshot (each one is a new object).
  $effect(() => {
    const st = paneInfo(pane)?.status;
    untrack(() => driver.update(st));
  });

  // Clicking in the panel (to read, not to select text) hands the keyboard
  // back to whatever had it: the terminal, or its input box.
  let wrap = $state<HTMLElement>();
  $effect(() => {
    const el = wrap;
    if (!el) return;
    let before: HTMLElement | null = null;
    const down = () => {
      const a = document.activeElement as HTMLElement | null;
      before = a && a !== document.body && !el.contains(a) ? a : null;
    };
    const up = () => {
      const sel = window.getSelection();
      if (sel && !sel.isCollapsed) return;
      const a = document.activeElement;
      if (before && a !== before && before.isConnected) before.focus({ preventScroll: true });
      else if (!before && (a === document.body || (a && el.contains(a)))) focusTerm(pane);
    };
    el.addEventListener("pointerdown", down, true);
    el.addEventListener("pointerup", up);
    return () => {
      el.removeEventListener("pointerdown", down, true);
      el.removeEventListener("pointerup", up);
    };
  });
</script>

<div class="term-files" bind:this={wrap}>
  <FilesPanel
    model={feed}
    {cwd}
    visible={onScreen}
    {overlay}
    {onclose}
    persistKey="term"
    defaultShare={0.5}
    minRest={300}
    actions={[{ icon: "⧉", title: "파일 창을 팝업 창으로 빼기 (여러 세션을 한 창에 모아 봅니다)", run: () => popOut(pane) }]}
  />
</div>

<style>
  /* No box of its own: the panel is the row's flex item. */
  .term-files {
    display: contents;
  }
</style>
