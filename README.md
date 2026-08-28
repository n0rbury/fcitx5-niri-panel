# fcitx5-niri-panel

A standalone Fcitx 5 Kimpanel-compatible candidate/preedit panel for Niri.

## Current stage

The panel implements the **panel side** of the KDE Kimpanel D-Bus protocol and
renders the input state as a bottom-anchored wlr-layer-shell bar on Niri.
Fcitx 5's Kimpanel UI addon sends panel updates (preedit, aux, candidates,
cursor rect) which are both logged and painted: candidates appear in a dark
bar at the bottom of the screen while composing, with the selected row
highlighted, and the bar hides again when input is committed or focus leaves.
Clicking a candidate row selects it: the panel emits the Kimpanel
`SelectCandidate` signal, which Fcitx answers by committing the candidate.
When the candidate list has more than one page (Rime's `has_next`), paging
arrows appear on the right edge of the bar; clicking them pages through the
list via the `LookupTablePageUp`/Down signals.

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

The protocol is verified end-to-end: for input contexts without the
`ClientSideInputPanel` capability (X11/XWayland apps such as Feishu, GTK/Qt
apps, synthetic D-Bus contexts), Fcitx routes candidate tables to this panel
via `SetLookupTable`. Wayland-native portal clients (e.g. Ghostty,
Chromium-class apps) keep the client-side UI path and do not reach the panel.

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
