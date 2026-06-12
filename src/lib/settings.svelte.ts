// User-tweakable UI settings, applied live and persisted in localStorage.
// (M5 moves persistence to config.toml; the live-application path stays.)

const KEY = "cmux.settings";

interface Settings {
  fontSize: number;
  sidebarWidth: number;
}

const DEFAULTS: Settings = { fontSize: 14, sidebarWidth: 230 };

function load(): Settings {
  try {
    return { ...DEFAULTS, ...JSON.parse(localStorage.getItem(KEY) ?? "{}") };
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
