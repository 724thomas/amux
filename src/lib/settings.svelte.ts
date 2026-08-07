// User-tweakable UI settings, applied live and persisted in localStorage.

const KEY = "amux.settings";
/// Pre-rename key; read once as a fallback so existing settings carry over.
const OLD_KEY = "cmux.settings";

interface Settings {
  fontSize: number;
  sidebarWidth: number;
  theme: string;
  /** Height (px) of the sidebar notification panel; drag-resizable. */
  notifHeight: number;
  /** Pin the last submitted command (up to 4 lines) inside each pane. */
  showLastInput: boolean;
  /** Draggable position (px from the pane's top-left) of the last-command chip. */
  lastInputPos: { x: number; y: number };
  /** Pin a prompt composer at the bottom of every pane (type without the
   *  terminal auto-scrolling to the newest output). */
  showComposer: boolean;
}

const DEFAULTS: Settings = {
  fontSize: 14,
  sidebarWidth: 230,
  theme: "tokyo-night",
  notifHeight: 200,
  showLastInput: true,
  lastInputPos: { x: 8, y: 6 },
  showComposer: true,
};

function load(): Settings {
  try {
    const raw = localStorage.getItem(KEY) ?? localStorage.getItem(OLD_KEY) ?? "{}";
    return { ...DEFAULTS, ...JSON.parse(raw) };
  } catch {
    return { ...DEFAULTS };
  }
}

export const settings = $state<Settings>(load());

function save() {
  localStorage.setItem(KEY, JSON.stringify(settings));
}

export function setFontSize(size: number) {
  settings.fontSize = Math.min(32, Math.max(8, Math.round(size)));
  save();
}

export function adjustFontSize(delta: number) {
  setFontSize(settings.fontSize + delta);
}

export function resetFontSize() {
  setFontSize(DEFAULTS.fontSize);
}

export function setSidebarWidth(width: number) {
  settings.sidebarWidth = Math.min(480, Math.max(140, Math.round(width)));
  save();
}

export function setTheme(id: string) {
  settings.theme = id;
  save();
}

export function setNotifHeight(px: number) {
  const max = Math.round(window.innerHeight * 0.7);
  settings.notifHeight = Math.min(max, Math.max(64, Math.round(px)));
  save();
}

export function toggleShowLastInput() {
  settings.showLastInput = !settings.showLastInput;
  save();
}

export function setShowComposer(on: boolean) {
  settings.showComposer = on;
  save();
}

export function toggleShowComposer() {
  setShowComposer(!(settings.showComposer ?? true));
}

/** Live position update while dragging the last-command chip (not persisted;
 *  call saveSettings() once on drag end to avoid a localStorage write per move). */
export function setLastInputPos(x: number, y: number) {
  settings.lastInputPos = { x: Math.round(x), y: Math.round(y) };
}

export function saveSettings() {
  save();
}
