# fcitx5-niri-panel

A standalone Fcitx 5 Kimpanel-compatible candidate/preedit panel for Niri.

## Current stage

The panel implements the **panel side** of the KDE Kimpanel D-Bus protocol and
renders the input state as an opaque, content-sized wlr-layer-shell bar on
Niri. Fcitx 5's Kimpanel UI addon sends panel updates (preedit, aux,
candidates, cursor rect) which are both logged and painted: while composing,
the bar shows the preedit line above the candidates — by default a single
horizontal row joining every candidate (Fcitx may also request the vertical
layout, rendered as stacked rows) — with a full-row-height highlight on the
selected candidate, and the bar hides again when input is committed or focus
leaves. Clicking a candidate selects it: the panel emits the Kimpanel
`SelectCandidate` signal, which Fcitx answers by committing the candidate.

The bar follows the caret for input contexts that report an absolute spot
rectangle (the X11/XWayland path, e.g. Feishu): it pins to the caret's output
and sits just below the caret, flipping above it when typing near the bottom
of the display. Wayland-native clients only report window-relative cursor
rects — and Fcitx's D-Bus frontend does not flag them as relative — while
niri's IPC exposes no global positions for tiled windows, so caret-accurate
placement is not reachable there; instead the bar anchors to the bottom of
the output holding the focused window, tracked live over the niri IPC event
stream. Multi-page candidate lists have no on-panel paging UI yet: the
`LookupTablePageUp`/`Down` signals are implemented, but nothing on the bar
invokes them.

The long-term design is:

```text
Fcitx 5 Kimpanel UI
        |
        | org.kde.impanel / org.kde.impanel2
        v
fcitx5-niri-panel
   |            |
 state        renderer
   |            |
   +-----+------+
         |
  Wayland popup / layer-shell
         |
        Niri
```

The protocol is verified end-to-end: Fcitx routes candidate tables to this
panel via `SetLookupTable` for every input context — X11/XWayland apps such
as Feishu, GTK/Qt apps, and synthetic D-Bus contexts unconditionally, and
Wayland-native portal clients (e.g. Ghostty, Chromium-class apps) once Fcitx
is started with `XDG_CURRENT_DESKTOP=GNOME:niri`. Fcitx's D-Bus frontend
only strips the client-side UI capability from Wayland input contexts on
GNOME-type desktops (`useClientSideUI`), and the `gnome` component of that
variable makes it do so under Niri.

`cargo run -- --headless` runs the panel without the Wayland bar (e.g. over
SSH). `cargo run --bin icdriver -- --text=nihao` drives a synthetic input
context for repeatable verification.

## Autostart

```bash
cargo build --release
./scripts/install-autostart.sh
```

installs the release binary to `~/.local/bin` and registers a systemd user
service (`fcitx5-niri-panel.service`) that starts the panel at login and
restarts it on failure. Logs land in the user journal:

```bash
journalctl --user -u fcitx5-niri-panel -f
```

## Build and run

```bash
cargo test
cargo run -- --verbose
```

Then, from another terminal:

```bash
./scripts/check-kimpanel.sh
```

With Fcitx5 restarted and the kimpanel addon enabled, the process should own:

```text
org.kde.impanel
```

and Fcitx should stop relying on the GNOME/KDE panel process that previously
owned that service.
