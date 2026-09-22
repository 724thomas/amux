<script lang="ts">
  // "정말 닫을까요?" — the one dialog in front of every tab or workspace close
  // (Ctrl+W, Ctrl+Shift+W, the tab's ×, middle-click, the sidebar menu, the
  // palette) and the window's own close. See `askClose` in state.svelte.ts.
  // "확인" has the focus when it opens, so Enter confirms.
  import { onMount } from "svelte";
  import { activePane, answerClose, closeConfirm, focusTerm } from "./state.svelte";

  let okButton = $state<HTMLButtonElement>();
  let cancelButton = $state<HTMLButtonElement>();

  function finish(ok: boolean) {
    answerClose(ok);
    // On cancel, hand the keyboard back to the terminal it came from. Deferred
    // so the keystroke that answered is fully spent before a terminal can see
    // it. On confirm the closing tab takes its focus with it, and the snapshot
    // that follows focuses whatever is on screen next.
    if (!ok) setTimeout(() => focusTerm(activePane()), 0);
  }

  // Captured at the window, ahead of every other listener, so while the dialog
  // is up no key reaches a terminal or fires a shortcut behind it. Enter and
  // Space are answered here on keydown (with preventDefault, which also
  // cancels the keypress xterm listens to) instead of being left to the
  // button's native activation: that is what guarantees the Enter that
  // confirms never lands in a terminal as a submitted line. Both press the
  // focused button, which is 확인 unless the user tabbed to 취소.
  function onKey(e: KeyboardEvent) {
    e.stopPropagation();
    if (e.isComposing || e.keyCode === 229) return;
    e.preventDefault();
    if (e.key === "Enter" || e.key === " ") {
      finish(document.activeElement !== cancelButton);
    } else if (e.key === "Escape") {
      finish(false);
    } else if (e.key === "Tab") {
      (document.activeElement === okButton ? cancelButton : okButton)?.focus();
    }
  }

  onMount(() => {
    okButton?.focus();
    // Whatever opened the dialog may still move the keyboard once more (the
    // palette focuses the terminal right after it runs an action), so claim
    // it again once that has happened.
    const refocus = setTimeout(() => okButton?.focus(), 0);
    window.addEventListener("keydown", onKey, true);
    return () => {
      clearTimeout(refocus);
      window.removeEventListener("keydown", onKey, true);
    };
  });
</script>

<div
  class="cc-backdrop"
  role="presentation"
  onpointerdown={(e) => {
    if (e.target === e.currentTarget) finish(false);
  }}
>
  <!-- Clicking the text must not take focus off the buttons: which button has
       focus is what Enter will press, and it has to stay visible. -->
  <div
    class="cc-card"
    role="alertdialog"
    aria-modal="true"
    aria-labelledby="cc-title"
    tabindex="-1"
    onmousedown={(e) => {
      if (!(e.target instanceof HTMLButtonElement)) e.preventDefault();
    }}
  >
    <h2 id="cc-title">{closeConfirm.title}</h2>
    <p class="cc-detail">{closeConfirm.detail}</p>
    <div class="cc-actions">
      <button class="cc-cancel" bind:this={cancelButton} onclick={() => finish(false)}>취소</button>
      <button class="cc-ok" bind:this={okButton} onclick={() => finish(true)}>확인</button>
    </div>
    <p class="cc-hint">Enter 확인 / Esc 취소</p>
  </div>
</div>

<style>
  .cc-backdrop {
    position: fixed;
    inset: 0;
    z-index: 10001;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px;
    background: color-mix(in srgb, #000 45%, transparent);
  }
  .cc-card {
    width: min(420px, 100%);
    padding: 20px 22px 14px;
    background: var(--surface-3);
    border: 1px solid var(--border);
    border-radius: 12px;
    box-shadow: 0 18px 48px rgba(0, 0, 0, 0.45);
  }
  .cc-card h2 {
    margin: 0 0 8px;
    font-size: 1rem;
    color: var(--text);
  }
  .cc-detail {
    margin: 0 0 16px;
    font-size: 0.82rem;
    line-height: 1.6;
    color: var(--muted);
  }
  .cc-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .cc-actions button {
    padding: 7px 18px;
    font: inherit;
    font-size: 0.85rem;
    border-radius: 7px;
    cursor: pointer;
  }
  .cc-cancel {
    color: var(--muted);
    background: transparent;
    border: 1px solid var(--border);
  }
  .cc-cancel:hover,
  .cc-cancel:focus-visible {
    color: var(--text);
    background: color-mix(in srgb, var(--text) 10%, transparent);
  }
  .cc-ok {
    font-weight: 700;
    color: var(--bg);
    background: var(--accent);
    border: 1px solid var(--accent);
  }
  /* The focused button is the one Enter presses, so make it unmistakable. */
  .cc-actions button:focus {
    outline: 2px solid color-mix(in srgb, var(--accent) 70%, var(--text));
    outline-offset: 2px;
  }
  .cc-hint {
    margin: 10px 0 0;
    font-size: 0.7rem;
    text-align: right;
    color: var(--muted);
  }
</style>
