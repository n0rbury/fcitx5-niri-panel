//! Tracks niri's focused window through the niri IPC socket, so the panel can
//! anchor its bar to the display the user is typing on.
//!
//! Niri does not expose global positions for tiled windows, so caret-accurate
//! placement is not reachable for native Wayland clients; the reliable signal
//! is which output holds the focused window. This watcher subscribes to
//! niri's event stream (newline-delimited JSON over `NIRI_SOCKET`) and keeps
//! the focused window's output name in shared state. Events are parsed
//! loosely (`serde_json::Value`) so niri can add fields without breaking us.
//! Niri's socket name embeds the compositor PID and changes on restart, so
//! the path is rediscovered on every reconnect.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Shared handle to the focused-window tracking state.
#[derive(Default)]
pub struct NiriFocus {
    output: Mutex<Option<String>>,
}

impl NiriFocus {
    /// Output name of the currently focused window, if known.
    pub fn output(&self) -> Option<String> {
        self.output.lock().expect("niri focus lock poisoned").clone()
    }

    /// Returns true when the stored output changed (caller should notify).
    fn set_output(&self, output: Option<String>) -> bool {
        let mut guard = self.output.lock().expect("niri focus lock poisoned");
        if *guard == output {
            return false;
        }
        *guard = output;
        true
    }
}

/// Tracking state reduced from the event stream.
#[derive(Default)]
struct NiriState {
    workspace_output: HashMap<u64, String>,
    window_workspace: HashMap<u64, Option<u64>>,
    focused_window: Option<u64>,
}

impl NiriState {
    fn focused_output(&self) -> Option<String> {
        let window = self.focused_window?;
        let workspace = self.window_workspace.get(&window)?.as_ref()?;
        self.workspace_output.get(workspace).cloned()
    }

    /// Apply one event-stream line; returns true when the focused output may
    /// have changed and should be recomputed.
    fn apply(&mut self, event: &serde_json::Value) -> bool {
        let Some((name, body)) = event.as_object().and_then(|object| object.iter().next()) else {
            return false;
        };
        match name.as_str() {
            "WorkspacesChanged" => {
                self.workspace_output.clear();
                if let Some(list) = body.get("workspaces").and_then(|v| v.as_array()) {
                    for workspace in list {
                        if let (Some(id), Some(output)) =
                            (workspace["id"].as_u64(), workspace["output"].as_str())
                        {
                            self.workspace_output.insert(id, output.to_string());
                        }
                    }
                }
                true
            }
            "WindowsChanged" => {
                self.window_workspace.clear();
                self.focused_window = None;
                if let Some(list) = body.get("windows").and_then(|v| v.as_array()) {
                    for window in list {
                        self.track_window(window);
                    }
                }
                true
            }
            "WindowOpenedOrChanged" => {
                self.track_window(&body["window"]);
                true
            }
            "WindowClosed" => {
                if let Some(id) = body["id"].as_u64() {
                    self.window_workspace.remove(&id);
                    if self.focused_window == Some(id) {
                        self.focused_window = None;
                    }
                }
                true
            }
            "WindowFocusChanged" => {
                self.focused_window = body["id"].as_u64();
                true
            }
            _ => false,
        }
    }

    fn track_window(&mut self, window: &serde_json::Value) {
        let Some(id) = window["id"].as_u64() else { return };
        self.window_workspace
            .insert(id, window["workspace_id"].as_u64());
        if window["is_focused"].as_bool().unwrap_or(false) {
            self.focused_window = Some(id);
        } else if self.focused_window == Some(id) {
            self.focused_window = None;
        }
    }
}

/// The socket to talk to: `NIRI_SOCKET`, or the newest `niri.*.sock` in the
/// runtime directory (covers niri restarts after this process started).
fn socket_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("NIRI_SOCKET") {
        return Some(PathBuf::from(path));
    }
    let runtime_dir = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?);
    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(&runtime_dir).ok()? {
        let Ok(entry) = entry else { continue };
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !(name.starts_with("niri.") && name.ends_with(".sock")) {
            continue;
        }
        let mtime = entry.metadata().and_then(|meta| meta.modified()).ok();
        let candidate = (mtime.unwrap_or(std::time::SystemTime::UNIX_EPOCH), entry.path());
        if newest.as_ref().map(|(t, _)| candidate.0 > *t).unwrap_or(true) {
            newest = Some(candidate);
        }
    }
    newest.map(|(_, path)| path)
}

/// Start the watcher thread; returns `None` (without side effects) when no
/// niri socket is reachable, in which case the renderer keeps its previous
/// behavior. Every focus change sends one unit over `notify`.
pub fn spawn(notify: Sender<()>) -> Option<Arc<NiriFocus>> {
    socket_path()?;
    let focus = Arc::new(NiriFocus::default());
    let handle = focus.clone();
    let started = std::thread::Builder::new()
        .name("niri-focus".into())
        .spawn(move || watcher_loop(handle, notify));
    started.ok()?;
    Some(focus)
}

fn watcher_loop(focus: Arc<NiriFocus>, notify: Sender<()>) {
    loop {
        if let Err(e) = run_once(&focus, &notify) {
            eprintln!("[niri] {e}");
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

fn run_once(focus: &NiriFocus, notify: &Sender<()>) -> io::Result<()> {
    let Some(path) = socket_path() else {
        return Err(io::Error::new(io::ErrorKind::NotFound, "no niri socket"));
    };
    let mut stream = UnixStream::connect(&path)?;
    // Request::EventStream serializes to the quoted string "EventStream".
    stream.write_all(b"\"EventStream\"\n")?;
    let mut reader = BufReader::new(stream);

    let mut line = String::new();
    reader.read_line(&mut line)?;
    if !line.contains("\"Ok\"") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("event stream refused: {}", line.trim()),
        ));
    }

    let mut state = NiriState::default();
    loop {
        line.clear();
        let read = reader.read_line(&mut line)?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "niri event stream closed",
            ));
        }
        let Ok(event) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
            continue;
        };
        if state.apply(&event) {
            let output = state.focused_output();
            if focus.set_output(output) {
                let _ = notify.send(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tracks_focus_across_outputs() {
        let mut state = NiriState::default();
        assert!(state.apply(&json!({"WorkspacesChanged":{"workspaces":[
            {"id":1,"idx":1,"output":"DP-1"},
            {"id":2,"idx":1,"output":"HDMI-A-2"},
        ]}})));
        assert!(state.apply(&json!({"WindowsChanged":{"windows":[
            {"id":5,"workspace_id":1,"is_focused":false},
            {"id":7,"workspace_id":2,"is_focused":true},
        ]}})));
        assert_eq!(state.focused_output().as_deref(), Some("HDMI-A-2"));

        assert!(state.apply(&json!({"WindowFocusChanged":{"id":5}})));
        assert_eq!(state.focused_output().as_deref(), Some("DP-1"));

        assert!(state.apply(&json!({"WindowFocusChanged":{"id":null}})));
        assert_eq!(state.focused_output(), None);
    }

    #[test]
    fn untracked_events_are_ignored() {
        let mut state = NiriState::default();
        assert!(!state.apply(&json!({"KeyboardLayoutSwitched":{"idx":0}})));
        assert!(!state.apply(&json!("not an object")));
        assert_eq!(state.focused_output(), None);
    }

    #[test]
    fn focus_change_to_unknown_window_clears_output() {
        let mut state = NiriState::default();
        state.apply(&json!({"WorkspacesChanged":{"workspaces":[
            {"id":1,"idx":1,"output":"DP-1"},
        ]}}));
        state.apply(&json!({"WindowsChanged":{"windows":[
            {"id":5,"workspace_id":1,"is_focused":true},
        ]}}));
        assert_eq!(state.focused_output().as_deref(), Some("DP-1"));
        state.apply(&json!({"WindowFocusChanged":{"id":99}}));
        assert_eq!(state.focused_output(), None);
    }
}
