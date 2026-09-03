# Focused-window positions from the niri IPC

Why and how this panel gets caret-accurate positioning for Wayland-native
apps, and what the local niri build changes for it.

## The problem

A standalone panel lives in its own process and draws on a layer-shell
surface in global compositor space. The Kimpanel channel tells it the caret
position, but only in the coordinate space the client has:

- XIM/XWayland clients (Feishu): absolute root coordinates. Directly usable.
- GTK/Qt clients with the fcitx IM modules (Ghostty, Nautilus):
  `SetRelativeSpotRectV2(x, y, w, h, scale)` — relative to the app's
  toplevel surface, in physical pixels, plus the client's scale factor
  (fcitx5-gtk computes it via `gtk_widget_translate_coordinates` +
  `gtk_native_get_surface_transform`).
- text-input-v3 clients (Electron, VS Code via the Wayland IM): nothing —
  the caret never reaches fcitx (fcitx5#1609); the compositor knows it but
  only shares it with the input-method client through popup anchoring.

For the second group the missing datum is the toplevel's global position.
The GNOME Kimpanel extension solves the identical problem by reading
`global.display.focus_window` in-process (panel.js `updatePosition`:
`focus_window position + relative rect × scale`). mutter hands its shell
that geometry; a niri-side panel needs the same datum from niri.

## The niri gap and the patch

`niri msg -j focused-window` (and `windows`, and the event stream) report a
`WindowLayout` with `tile_pos_in_workspace_view` — but niri fills it only
for floating windows; tiled windows get `null`, even though the scrolling
layout computes those positions internally for rendering
(`ScrollingSpace::tiles_with_render_positions`).

`niri-patch/0001-ipc-fill-tile_pos_in_workspace_view-for-tiled-windows.patch`
(v2 lines against the v26.04 tag) fills the field for tiled windows using
the same math the renderer uses: `column_xs` + tile offset within the
column, minus the settled view scroll (`view_offset.target()` — no animated
offsets, mirroring the floating layout's IPC policy of avoiding per-frame
spam), rounded to physical pixels.

With it, for the focused window:

```json
"layout": { "tile_pos_in_workspace_view": [1258.0, 42.0],
            "window_offset_in_tile": [0.0, 0.0], ... }
```

Coordinate semantics (verified against the niri v26.04 source and a nested
session): `tile_pos + window_offset_in_tile` is the window's visual-geometry
origin relative to its output — the workspace render geometry origin is
(0,0) for the active workspace, outer gaps and layer-shell struts are
already included (tiles_origin adds `working_area.loc + gaps`; column x
alignment goes through `view_offset`). So:

    global_caret = output_pos + tile_pos + window_offset_in_tile
                   + relative_rect / scale

with no further correction **once the rects are content-relative**. They
are not, stock: fcitx5-gtk4 reports toplevel-surface coordinates — it adds
`gtk_native_get_surface_transform`, the app's CSD shadow margin
(`gtk/gtkwindow.c` shadow extents, 0 when maximized/fullscreen), and its
popover chain accumulates parent-surface offsets — so consumers anchoring
to the content origin double-count the margin (~28px low in GNOME
TextEditor, ~31px in Nautilus's F2 rename popover, 0 in Ghostty only
because its client disables window shadows). The correction lives in the
client, not the panel: `fcitx5-gtk-patch/` makes the GTK4 IM module convert
the rect to content coordinates at the end of its chain (and the same
oracle-app calibration that proved it is documented there).

## Panel side

`src/niri.rs` runs one cached IPC resolution — `niri msg -j focused-window`
for the tile position plus `niri msg -j workspaces` to map the window's
workspace to its output name (output-local tile coordinates are ambiguous
otherwise: a point inside one output's local bounds may fit another's too,
which sent the bar to the wrong display until this was resolved by name) —
and converts the rect per the formula above. `src/render.rs` then treats it
exactly like an absolute XIM spot (pin to the window's output, below/above
the caret). Any failure — other compositor, niri without the patch, no
focused window — leaves the rect relative and the bottom-anchor fallback
applies.

## Deployment

```bash
# niri: patch + build + install + drop-in (takes effect at next login)
git apply niri-patch/0001-*.patch        # in a niri v26.04 checkout
cargo build --release
install -D target/release/niri ~/.local/bin/niri
mkdir -p ~/.config/systemd/user/niri.service.d
printf '[Service]\nExecStart=\nExecStart=%s\n' "$HOME/.local/bin/niri --session" \
  > ~/.config/systemd/user/niri.service.d/override.conf
systemctl --user daemon-reload
```

`niri --version` identifies the build as `26.04 (v26.04-modified)`. Removing
the drop-in and `daemon-reload` reverts to the distro compositor; the patch
itself is upstreamable as-is.
