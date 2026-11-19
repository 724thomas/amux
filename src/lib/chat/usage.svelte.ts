// What the chat headers' meter needs beyond one conversation: the Claude
// subscription usage, read from amux's own poll file on one shared timer
// while any chat view is open, and the context window size of each model
// seen, kept across launches.

import { untrack } from "svelte";
import { claudeUsage, type ClaudeUsage } from "../ipc";

/** The file changes every five minutes; "time left" moves every minute. */
const TICK_MS = 30_000;

export const usageFeed = $state<{ usage: ClaudeUsage | null; nowSec: number }>({
  usage: null,
  nowSec: Math.floor(Date.now() / 1000),
});

let watchers = 0;
let timer: ReturnType<typeof setInterval> | null = null;

async function refresh() {
  try {
    usageFeed.usage = await claudeUsage();
  } catch {
    usageFeed.usage = null;
  }
  usageFeed.nowSec = Math.floor(Date.now() / 1000);
}

/** Keep `usageFeed` current while the caller is mounted. Call it from an
 *  `$effect` and return what it returns: the last view to go stops the timer. */
export function watchUsage(): () => void {
  untrack(() => {
    if (watchers++ === 0) {
      void refresh();
      timer = setInterval(() => void refresh(), TICK_MS);
    }
  });
  return () => {
    if (--watchers === 0 && timer !== null) {
      clearInterval(timer);
      timer = null;
    }
  };
}

// -- context windows by model -------------------------------------------------

const WINDOWS_KEY = "amux.ctxWindows";

function loadWindows(): Record<string, number> {
  try {
    const v = JSON.parse(localStorage.getItem(WINDOWS_KEY) ?? "{}");
    return v && typeof v === "object" && !Array.isArray(v) ? v : {};
  } catch {
    return {};
  }
}

/** Model id → context window, as turns' `result`s reported it. A window is
 *  only known once a turn ends; this lets a chat on a model seen before show
 *  its `[ctx]` right away (a conversation taken over from the terminal, the
 *  first reply of a new one). */
const windows = $state<Record<string, number>>(loadWindows());

export function rememberWindow(model: string | null, window: number | null) {
  if (!model || !window || windows[model] === window) return;
  windows[model] = window;
  try {
    localStorage.setItem(WINDOWS_KEY, JSON.stringify(windows));
  } catch {
    // Storage full or off: the window is still known until the app closes.
  }
}

export function knownWindow(model: string | null): number | null {
  return (model && windows[model]) || null;
}
