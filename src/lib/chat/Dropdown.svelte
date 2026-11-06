<script lang="ts">
  // A small picker the app draws itself, in the look of the sidebar's theme
  // picker. A native <select> opens a list drawn by GTK under WebKitGTK, which
  // ignores the app's colors (a white list with the theme's light text on it),
  // so the chat header uses this instead.
  interface Option {
    value: string;
    label: string;
  }

  let {
    value,
    options,
    placeholder = "",
    disabled = false,
    title = "",
    onchange,
  }: {
    value: string;
    options: Option[];
    /** Shown while `value` matches no option. */
    placeholder?: string;
    disabled?: boolean;
    title?: string;
    onchange: (value: string) => void;
  } = $props();

  let open = $state(false);
  let root = $state<HTMLElement>();
  const current = $derived(options.find((o) => o.value === value));

  function choose(v: string) {
    open = false;
    if (v !== value) onchange(v);
  }

  // Open: a click anywhere else, or Esc, closes it. Esc stops here so it does
  // not also reach the conversation's input (where it would stop Claude).
  $effect(() => {
    if (!open) return;
    const down = (e: PointerEvent) => {
      if (root && !root.contains(e.target as Node)) open = false;
    };
    const key = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      e.stopPropagation();
      open = false;
    };
    window.addEventListener("pointerdown", down, true);
    window.addEventListener("keydown", key, true);
    return () => {
      window.removeEventListener("pointerdown", down, true);
      window.removeEventListener("keydown", key, true);
    };
  });
  $effect(() => {
    if (disabled) open = false;
  });
</script>

<div class="dd" bind:this={root}>
  <button
    class="dd-btn"
    {title}
    {disabled}
    aria-haspopup="listbox"
    aria-expanded={open}
    onmousedown={(e) => e.preventDefault()}
    onclick={() => (open = !open)}
  >
    <span class="dd-label">{current?.label ?? (value || placeholder)}</span>
    <span class="dd-caret">▾</span>
  </button>
  {#if open}
    <div class="dd-menu" role="listbox">
      {#each options as o (o.value)}
        <button
          role="option"
          aria-selected={o.value === value}
          class:selected={o.value === value}
          onmousedown={(e) => e.preventDefault()}
          onclick={() => choose(o.value)}>{o.label}</button
        >
      {/each}
    </div>
  {/if}
</div>

<style>
  .dd {
    position: relative;
    display: inline-flex;
    min-width: 0;
  }
  .dd-btn {
    display: flex;
    align-items: center;
    gap: 6px;
    max-width: 13em;
    padding: 2px 8px;
    font: inherit;
    color: var(--text);
    background: var(--surface-3, var(--surface-2));
    border: 1px solid var(--border-2);
    border-radius: 5px;
    cursor: pointer;
  }
  .dd-btn:hover:not(:disabled) {
    border-color: var(--accent);
  }
  .dd-btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .dd-label {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dd-caret {
    color: var(--muted);
    font-size: 0.85em;
  }
  .dd-menu {
    position: absolute;
    top: calc(100% + 2px);
    left: 0;
    min-width: 100%;
    max-height: 60vh;
    overflow-y: auto;
    z-index: 1000;
    display: flex;
    flex-direction: column;
    padding: 0.25rem;
    background: var(--surface-2);
    border: 1px solid var(--border-2);
    border-radius: 6px;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.5);
  }
  .dd-menu button {
    display: block;
    padding: 0.35rem 0.5rem;
    text-align: left;
    white-space: nowrap;
    font: inherit;
    color: var(--text);
    background: none;
    border: none;
    border-radius: 4px;
    cursor: pointer;
  }
  .dd-menu button:hover {
    background: var(--border-2);
  }
  .dd-menu button.selected {
    color: var(--accent);
    font-weight: 700;
  }
</style>
