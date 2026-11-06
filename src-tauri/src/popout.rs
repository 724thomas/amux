//! The files pop-out window: the files panels of several panes moved out of
//! the main window into one separate window (label "files") that shows them
//! side by side. Which panes are out lives here, so both windows see the same
//! list: every change is broadcast as `files:state`. Closing the pop-out puts
//! every panel back where it came from.
//!
//! The window decisions (`on_destroyed`, `tracks_focus`) are plain functions
//! because getting them wrong is fatal: the app shuts down when its main window
//! is destroyed, and a second window must not trigger that.

use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

pub const MAIN: &str = "main";
pub const FILES: &str = "files";

#[derive(Debug, Default, Clone, Serialize, PartialEq)]
pub struct PopoutState {
    /// Panes whose files panel is in the pop-out window, in the order they came.
    pub panes: Vec<String>,
    /// The main window's look (theme id, font size), for the pop-out to match.
    pub look: Option<serde_json::Value>,
}

#[derive(Default)]
pub struct Popout {
    inner: Mutex<PopoutState>,
}

impl Popout {
    fn update(&self, f: impl FnOnce(&mut PopoutState)) -> PopoutState {
        let mut s = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut s);
        s.clone()
    }

    pub fn state(&self) -> PopoutState {
        self.update(|_| {})
    }

    /// Move a pane's panel out (`out`) or back in. Returns the new state.
    pub fn set(&self, pane: &str, out: bool, look: Option<serde_json::Value>) -> PopoutState {
        self.update(|s| {
            if let Some(look) = look {
                s.look = Some(look);
            }
            if out {
                if !s.panes.iter().any(|p| p == pane) {
                    s.panes.push(pane.to_string());
                }
            } else {
                s.panes.retain(|p| p != pane);
            }
        })
    }

    /// The pop-out window went away: every panel is back in its pane.
    pub fn clear_panes(&self) -> PopoutState {
        self.update(|s| s.panes.clear())
    }

    pub fn set_look(&self, look: serde_json::Value) -> PopoutState {
        self.update(|s| s.look = Some(look))
    }
}

/// What a destroyed window means for the app.
#[derive(Debug, PartialEq)]
pub enum OnDestroyed {
    /// The main window: stop every chat and the engine (and close the pop-out).
    ShutDown,
    /// The pop-out: its panels go back to their panes.
    PanelsBack,
    Nothing,
}

pub fn on_destroyed(label: &str) -> OnDestroyed {
    match label {
        MAIN => OnDestroyed::ShutDown,
        FILES => OnDestroyed::PanelsBack,
        _ => OnDestroyed::Nothing,
    }
}

/// Whether a window's focus changes reach the engine, which stays quiet about
/// the pane the user is looking at in the main window.
pub fn tracks_focus(label: &str) -> bool {
    label == MAIN
}

pub fn broadcast(app: &AppHandle, state: &PopoutState) {
    let _ = app.emit("files:state", state);
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn open_or_focus(app: &AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window(FILES) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(());
    }
    WebviewWindowBuilder::new(app, FILES, WebviewUrl::App("index.html".into()))
        .title("amux 파일 보기")
        .inner_size(1280.0, 820.0)
        .min_inner_size(480.0, 320.0)
        .build()
        .map(|_| ())
        .map_err(err)
}

/// Move a pane's files panel into the pop-out window (opening it) or back.
/// The last panel going back closes the window.
#[tauri::command]
pub async fn files_pop(
    app: AppHandle,
    pane: String,
    out: bool,
    look: Option<serde_json::Value>,
) -> Result<PopoutState, String> {
    let state = app.state::<Popout>().set(&pane, out, look);
    if out {
        if let Err(e) = open_or_focus(&app) {
            // No window to show it in: the panel stays where it was. Logged
            // here, since the main window only shows it in its console.
            tracing::warn!("files pop-out window did not open: {e}");
            let back = app.state::<Popout>().set(&pane, false, None);
            broadcast(&app, &back);
            return Err(e);
        }
    } else if state.panes.is_empty() {
        if let Some(w) = app.get_webview_window(FILES) {
            let _ = w.close();
        }
    }
    broadcast(&app, &state);
    Ok(state)
}

#[tauri::command]
pub async fn files_state(app: AppHandle) -> Result<PopoutState, String> {
    Ok(app.state::<Popout>().state())
}

/// The main window's theme or font size changed.
#[tauri::command]
pub async fn files_look(app: AppHandle, look: serde_json::Value) -> Result<(), String> {
    let state = app.state::<Popout>().set_look(look);
    broadcast(&app, &state);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_main_window_shuts_the_app_down() {
        assert_eq!(on_destroyed("main"), OnDestroyed::ShutDown);
        assert_eq!(on_destroyed("files"), OnDestroyed::PanelsBack);
        assert_eq!(on_destroyed("anything-else"), OnDestroyed::Nothing);
    }

    #[test]
    fn only_the_main_window_reports_focus() {
        assert!(tracks_focus("main"));
        assert!(!tracks_focus("files"));
    }

    #[test]
    fn panels_go_out_once_come_back_and_all_return_on_close() {
        let p = Popout::default();
        assert_eq!(p.set("a", true, None).panes, vec!["a"]);
        assert_eq!(p.set("b", true, None).panes, vec!["a", "b"]);
        assert_eq!(p.set("a", true, None).panes, vec!["a", "b"], "a pane goes out once");
        assert_eq!(p.set("a", false, None).panes, vec!["b"]);
        let look = serde_json::json!({ "theme": "tokyo-night", "fontSize": 14 });
        assert_eq!(p.set("c", true, Some(look.clone())).look, Some(look.clone()));
        assert_eq!(p.set("c", false, None).look, Some(look), "the look stays");
        assert!(p.clear_panes().panes.is_empty());
    }
}
