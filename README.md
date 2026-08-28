# fcitx5-niri-panel

A standalone Fcitx 5 Kimpanel-compatible candidate/preedit panel for Niri.

## Current stage

The prototype implements the **panel side** of the KDE Kimpanel D-Bus
protocol. Fcitx 5's Kimpanel UI addon is the client and sends panel updates to
this process.

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

The first milestone is deliberately headless: prove that Fcitx discovers the
standalone panel and delivers real candidate-table and cursor-position data.

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
