#!/usr/bin/env bash
#
# Build and install amux on macOS.
#
#   - Builds the frontend, the Tauri app bundle (amux.app + .dmg), and the
#     `amux` CLI (release).
#   - Copies amux.app into /Applications.
#   - Installs the `amux` CLI onto your PATH so panes can drive amux
#     (mirrors the Linux .deb, which drops `amux` into /usr/bin).
#
# Usage:
#   scripts/install-macos.sh              # build + install app and CLI
#   AMUX_BIN_DIR=~/.local/bin scripts/install-macos.sh   # custom CLI dir
#   scripts/install-macos.sh --build-only # build only, no install
#
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$(pwd)"
BIN_DIR="${AMUX_BIN_DIR:-/usr/local/bin}"
BUILD_ONLY=0
[ "${1:-}" = "--build-only" ] && BUILD_ONLY=1

echo "==> Checking toolchain"
command -v cargo >/dev/null || { echo "cargo not found — install Rust: https://rustup.rs"; exit 1; }
command -v bun   >/dev/null || { echo "bun not found — install: brew install oven-sh/bun/bun"; exit 1; }

echo "==> Installing JS deps"
bun install

echo "==> Building amux CLI (release)"
cargo build --release -p amux-cli

echo "==> Building amux.app"
# Only the .app is needed to install locally. Skipping the .dmg target avoids
# the slow (and, in headless/automation contexts, hang-prone) Finder/AppleScript
# window-styling step in tauri's dmg bundler. For a distributable .dmg, run
# `bun run tauri build` (both targets are declared in tauri.macos.conf.json).
bun run tauri build --bundles app

APP="$ROOT/target/release/bundle/macos/amux.app"
CLI="$ROOT/target/release/amux"
[ -d "$APP" ] || { echo "app bundle not found at $APP"; exit 1; }
[ -x "$CLI" ] || { echo "CLI binary not found at $CLI"; exit 1; }

if [ "$BUILD_ONLY" = "1" ]; then
  echo "==> Build complete (no install requested)"
  echo "    app: $APP"
  echo "    cli: $CLI"
  exit 0
fi

echo "==> Installing amux.app → /Applications"
rm -rf "/Applications/amux.app"
cp -R "$APP" "/Applications/amux.app"

echo "==> Installing amux CLI → $BIN_DIR"
if [ -w "$BIN_DIR" ] || [ ! -e "$BIN_DIR" -a -w "$(dirname "$BIN_DIR")" ]; then
  mkdir -p "$BIN_DIR"
  install -m 0755 "$CLI" "$BIN_DIR/amux"
else
  echo "    (need sudo to write $BIN_DIR)"
  sudo mkdir -p "$BIN_DIR"
  sudo install -m 0755 "$CLI" "$BIN_DIR/amux"
fi

echo
echo "==> Done."
echo "    App:  /Applications/amux.app  (also available in Launchpad)"
echo "    CLI:  $BIN_DIR/amux"
case ":$PATH:" in
  *":$BIN_DIR:"*) : ;;
  *) echo "    NOTE: $BIN_DIR is not on your PATH — add it so panes can find 'amux'." ;;
esac
echo
echo "    First launch is unsigned: right-click amux.app → Open, or run:"
echo "      xattr -dr com.apple.quarantine /Applications/amux.app"
