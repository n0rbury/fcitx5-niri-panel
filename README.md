# fcitx5-niri-panel

> **Status: pivoted (2026-09).** The standalone panel is retired from active
> use. The final setup is stock fcitx5 with a custom theme
> (`fcitx5-theme/Noctalia/`) and the fcitx wiki's Wayland environment
> (no `GTK_IM_MODULE`/`QT_IM_MODULE`; `XMODIFIERS=@im=fcitx` for XWayland):
> text-input-v3 apps get fcitx5's ClassicUI popup anchored by niri's
> input-method-v2 support, XWayland/XIM apps get the themed xcb popup —
> positioning correct by construction everywhere, no runtime patches
> required. The panel source, the patches under `niri-patch/`,
> `fcitx5-gtk-patch/` and `fcitx5-patch/`, and the coordinate-semantics
> findings in `docs/` are kept as the record of why, and are individually
> upstreamable.

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

The bar follows the caret. XWayland/XIM clients (e.g. Feishu) report an
absolute spot rectangle: the bar pins to the caret's output and sits just
below the caret, flipping above it near the bottom of the display.
Wayland-native clients only report window-relative cursor rects, so the
panel resolves them to global positions with the focused window's position
from the niri IPC (`niri msg -j focused-window` + `workspaces` for its
output) — the same relative-rect +
shell-known-window-position scheme as the GNOME Kimpanel extension. Stock
niri omits tile positions for tiled windows over IPC; the local niri build
adds them (see `niri-patch/`, deployed at `~/.local/bin/niri` via a
`niri.service.d` drop-in). GTK4 IM modules also report caret rects in
surface-buffer coordinates — one CSD shadow margin below the caret; the
local `fcitx5-frontend-gtk4` build carries the one-block fix in
`fcitx5-gtk-patch/` (content-relative rects on Wayland). Without a
resolvable rect (other compositors,
Electron-style clients that never report a caret to fcitx), the bar falls
back to the bottom of the compositor's active output: at each
hidden -> visible transition the layer surface is recreated without an
output binding, and niri maps output-less layer surfaces to its
then-active output. Multi-page candidate lists have no
on-panel paging UI yet: the `LookupTablePageUp`/`Down` signals are
implemented, but nothing on the bar invokes them.

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

Rendering follows the GNOME Kimpanel extension's design principle: use the
desktop shell's own toolkit and popup-menu styling for the IME UI, keeping
the panel's own styling minimal — port Fcitx's visual theme, and Fcitx's
classic candidate window remains the fallback for native Wayland clients
whose popup Niri already anchors and renders.

The panel consumes the Kimpanel channel plus one cached niri IPC resolution
(the focused window's position and output; see above). GTK/Qt and sandboxed
(portal) Wayland apps ship Fcitx IM modules that can draw candidates
themselves (`ClientSideInputPanel`); on-machine evaluation showed Fcitx sends
their UI state unicast to the app itself, so a panel cannot observe it. To
render those apps through this panel instead, Fcitx is configured to strip
that capability — `XDG_CURRENT_DESKTOP=GNOME:niri` on the Fcitx5 autostart
unit (fcitx5's supported `useClientSideUI` mechanism; see the
`app-org.fcitx.Fcitx5@autostart.service.d/override.conf` drop-in). With it,
every input context feeds the Kimpanel channel and the bar is the single
candidate UI across Wayland and XWayland.

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
