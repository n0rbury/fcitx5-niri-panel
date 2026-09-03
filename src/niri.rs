//! Focused-window geometry from the niri IPC.
//!
//! The panel is a standalone process, so the Kimpanel channel alone cannot
//! carry a Wayland-native app's global caret position: GTK/Qt clients report
//! the caret relative to their own toplevel surface, and only the compositor
//! knows where that toplevel is. niri's IPC is the compositor's shell-data
//! channel — the role mutter plays for the GNOME kimpanel extension, which
//! computes global caret = focus_window position + relative rect
//! (panel.js updatePosition). This module queries the focused window's
//! position together with the name of the output showing its workspace
//! (`niri msg -j focused-window` + `niri msg -j workspaces`, one cached
//! resolution) and converts Kimpanel's relative spot rects
//! (SetRelativeSpotRectV2: toplevel-surface-relative physical pixels plus
//! the client's scale factor) into output-relative global ones.
//!
//! On any failure — another compositor, niri without tile positions in the
//! IPC, no focused window — the caller keeps its bottom-anchor fallback.

use crate::model::Rect;

/// Geometry of the focused window in the coordinate spaces reported by
/// `niri msg -j focused-window`.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowGeo {
    /// `tile_pos_in_workspace_view`: position of the window's tile
    /// (including niri decorations) within the workspace view. Output-relative
    /// for the focused workspace: the workspace render geometry origin is
    /// (0, 0) there (monitor.rs workspaces_render_geo centers only the
    /// workspace-switch animation gap, which is 0 for the active workspace).
    pub tile_pos: (f64, f64),
    /// `window_offset_in_tile`: location of the window's visual geometry
    /// within its tile (includes niri border sizes).
    pub offset_in_tile: (f64, f64),
    /// Name of the output showing the window's workspace. Output-relative
    /// tile coordinates only become unambiguous once the owning output is
    /// known; guessing it by testing containment against other outputs'
    /// local bounds puts the bar on the wrong display whenever the point
    /// fits more than one output's local size.
    pub output: String,
}

impl WindowGeo {
    /// Convert a Kimpanel relative spot rect (toplevel-surface-relative,
    /// physical pixels, `scale` = the client's scale factor) into an
    /// output-relative logical rect. The caller resolves which output
    /// contains it and adds that output's logical position for global
    /// coordinates.
    pub fn absolute_spot(&self, spot: Rect, scale: f64) -> Rect {
        let scale = if scale > 0.0 { scale } else { 1.0 };
        let ox = self.tile_pos.0 + self.offset_in_tile.0;
        let oy = self.tile_pos.1 + self.offset_in_tile.1;
        Rect {
            x: (ox + spot.x as f64 / scale).round() as i32,
            y: (oy + spot.y as f64 / scale).round() as i32,
            width: (spot.width as f64 / scale).round() as i32,
            height: (spot.height as f64 / scale).round() as i32,
        }
    }
}

/// Focused-window parts of the `niri msg -j focused-window` payload.
#[derive(Debug, Clone, Copy, PartialEq)]
struct FocusedWindow {
    tile_pos: (f64, f64),
    offset_in_tile: (f64, f64),
    workspace_id: u64,
}

/// Parse the JSON output of `niri msg -j focused-window`. Returns None when
/// the window carries no tile position (niri reports null for tiled windows
/// without the tile-position IPC) or the shape is unexpected.
fn parse_focused_window(json: &str) -> Option<FocusedWindow> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let layout = v.get("layout")?;
    let pos = layout.get("tile_pos_in_workspace_view")?.as_array()?;
    if pos.len() != 2 {
        return None;
    }
    let off = layout.get("window_offset_in_tile")?.as_array()?;
    if off.len() != 2 {
        return None;
    }
    Some(FocusedWindow {
        tile_pos: (pos[0].as_f64()?, pos[1].as_f64()?),
        offset_in_tile: (off[0].as_f64()?, off[1].as_f64()?),
        workspace_id: v.get("workspace_id")?.as_u64()?,
    })
}

/// Name of the output showing the given workspace, from the JSON output of
/// `niri msg -j workspaces`.
fn parse_workspace_output(json: &str, workspace_id: u64) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    v.as_array()?
        .iter()
        .find(|ws| ws.get("id").and_then(|id| id.as_u64()) == Some(workspace_id))
        .and_then(|ws| ws.get("output").and_then(|o| o.as_str()))
        .map(str::to_string)
}

/// Query the running niri for the focused window's geometry: the window's
/// tile position plus the name of the output showing its workspace (one
/// `focused-window` and one `workspaces` read; the pair forms a single
/// cached resolution). A local Unix-socket round trip through the niri CLI,
/// cheap at typing rates; called only while a relative spot needs resolving.
///
/// Successful resolutions are cached briefly: every repaint of a visible bar
/// re-resolves even when the spot rect is unchanged (candidate selection
/// updates fire far faster than the caret moves), which otherwise spawns the
/// subprocess dozens of times per second. Bounding the entry's age keeps a
/// stale window position from outliving a focus change by more than the TTL.
pub fn focused_window() -> Option<WindowGeo> {
    const TTL: std::time::Duration = std::time::Duration::from_millis(300);
    static CACHE: std::sync::Mutex<Option<(std::time::Instant, WindowGeo)>> =
        std::sync::Mutex::new(None);

    let mut cache = CACHE.lock().ok()?;
    if let Some((at, win)) = cache.as_ref() {
        if at.elapsed() < TTL {
            return Some(win.clone());
        }
    }
    let out = std::process::Command::new("niri")
        .args(["msg", "-j", "focused-window"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let win = parse_focused_window(std::str::from_utf8(&out.stdout).ok()?)?;
    let ws = std::process::Command::new("niri")
        .args(["msg", "-j", "workspaces"])
        .output()
        .ok()?;
    if !ws.status.success() {
        return None;
    }
    let output = parse_workspace_output(std::str::from_utf8(&ws.stdout).ok()?, win.workspace_id)?;
    let geo = WindowGeo {
        tile_pos: win.tile_pos,
        offset_in_tile: win.offset_in_tile,
        output,
    };
    *cache = Some((std::time::Instant::now(), geo.clone()));
    Some(geo)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Fixture: verbatim output of niri 26.04 `niri msg -j focused-window`
    // on this machine, plus the tile position the patched layout fills in.
    const PATCHED: &str = r#"{"id":5,"title":"term","app_id":"com.mitchellh.ghostty","pid":1,
        "workspace_id":1,"is_focused":true,"is_floating":false,"is_urgent":false,
        "layout":{"pos_in_scrolling_layout":[2,1],"tile_size":[1072.0,1878.0],
        "window_size":[1072,1878],"tile_pos_in_workspace_view":[1258.0,42.0],
        "window_offset_in_tile":[0.0,0.0]},
        "focus_timestamp":{"secs":1,"nanos":0}}"#;

    // Fixture: verbatim shape of `niri msg -j workspaces` on this machine.
    const WORKSPACES: &str = r#"[{"id":1,"idx":1,"output":"DP-1","is_activated":true,
        "is_focused":false,"active_window_id":5},{"id":2,"idx":1,"output":"HDMI-A-2",
        "is_activated":true,"is_focused":true,"active_window_id":7}]"#;

    #[test]
    fn parses_patched_focused_window() {
        let win = parse_focused_window(PATCHED).expect("parses");
        assert_eq!(win.tile_pos, (1258.0, 42.0));
        assert_eq!(win.offset_in_tile, (0.0, 0.0));
        assert_eq!(win.workspace_id, 1);
    }

    #[test]
    fn maps_workspace_to_output() {
        assert_eq!(parse_workspace_output(WORKSPACES, 1), Some("DP-1".into()));
        assert_eq!(
            parse_workspace_output(WORKSPACES, 2),
            Some("HDMI-A-2".into())
        );
        assert_eq!(parse_workspace_output(WORKSPACES, 9), None);
    }

    #[test]
    fn rejects_unpatched_tiled_window() {
        // Stock niri 26.04 reports null for tiled windows.
        let json = r#"{"id":5,"layout":{"pos_in_scrolling_layout":[2,1],
            "tile_size":[1072.0,1878.0],"window_size":[1072,1878],
            "tile_pos_in_workspace_view":null,"window_offset_in_tile":[0.0,0.0]}}"#;
        assert_eq!(parse_focused_window(json), None);
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(parse_focused_window("Error: no focused window"), None);
        assert_eq!(parse_focused_window(""), None);
        assert_eq!(parse_focused_window(r#"{"layout":{}}"#), None);
        assert_eq!(parse_workspace_output("nope", 1), None);
    }

    #[test]
    fn converts_relative_spot_to_output_relative() {
        let win = WindowGeo {
            tile_pos: (1258.0, 42.0),
            offset_in_tile: (0.0, 0.0),
            output: "DP-1".into(),
        };
        // A GTK4-style caret line: logical (360, 31), zero width, 34 high.
        let spot = Rect { x: 360, y: 31, width: 0, height: 34 };
        assert_eq!(
            win.absolute_spot(spot, 1.0),
            Rect { x: 1618, y: 73, width: 0, height: 34 }
        );
        // Physical-pixel rect on a scale-2 output: divide by the scale.
        assert_eq!(
            win.absolute_spot(Rect { x: 720, y: 62, width: 0, height: 68 }, 2.0),
            Rect { x: 1618, y: 73, width: 0, height: 34 }
        );
    }

    #[test]
    fn honors_window_offset_in_tile() {
        // A bordered tile: the window sits inside the tile at the border size.
        let win = WindowGeo {
            tile_pos: (100.0, 50.0),
            offset_in_tile: (2.0, 2.0),
            output: "DP-1".into(),
        };
        let spot = Rect { x: 10, y: 20, width: 0, height: 34 };
        assert_eq!(
            win.absolute_spot(spot, 1.0),
            Rect { x: 112, y: 72, width: 0, height: 34 }
        );
    }

    #[test]
    fn guards_against_zero_scale() {
        let win = WindowGeo {
            tile_pos: (10.0, 20.0),
            offset_in_tile: (0.0, 0.0),
            output: "DP-1".into(),
        };
        let spot = Rect { x: 10, y: 10, width: 0, height: 34 };
        // Degenerate scale must not divide by zero; fall back to 1.0.
        assert_eq!(
            win.absolute_spot(spot, 0.0),
            Rect { x: 20, y: 30, width: 0, height: 34 }
        );
    }
}
