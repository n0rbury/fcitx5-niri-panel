# fcitx5-niri-panel

A compositor-independent Fcitx5 candidate UI designed for Niri and other Wayland compositors.

## Status

**Research / architecture spike.** No implementation yet.

The first goal is to determine the cleanest way to provide a reliable Fcitx 5 candidate/preedit UI on Niri, especially for XWayland applications.

## Problem

Fcitx 5 can expose input-method state through native Wayland protocols, while the KDE Kimpanel ecosystem provides a D-Bus-based UI protocol. Niri already has native handling for input-method popups, but real-world behavior can still be problematic around XWayland applications, positioning, scaling, and popup management.

This project investigates whether a small standalone panel can provide a more reliable UI without depending on GNOME Shell.

## Design principles

- Standalone process; no GNOME Shell dependency.
- Separate protocol/state handling from rendering.
- Prefer standard Wayland protocols and existing Niri behavior.
- Keep Kimpanel compatibility as an option, not an architectural assumption.
- Treat HiDPI, fractional scaling, multiple monitors, and XWayland as first-class concerns.
- Keep the initial implementation as small as possible.

## Proposed architecture

```text
                         Fcitx 5
                            |
                  +---------+----------+
                  |                    |
           native Wayland         Kimpanel D-Bus
            IME protocol          compatibility
                  |                    |
                  +---------+----------+
                            |
                   fcitx5-niri-panel
                    /               \
             protocol/state       renderer
                                  |
                            Wayland surface
                                  |
                                 Niri
```

The architecture spike will decide which input path is actually needed. The renderer should not know whether its state came from native Wayland IME integration or Kimpanel.

## First milestone

Display an Fcitx 5 candidate list at the correct spot in a Niri session, including when the focused application is running through XWayland.

## Related projects

- [Fcitx 5](https://github.com/fcitx/fcitx5)
- [GNOME Shell Kimpanel extension](https://github.com/wengxt/gnome-shell-extension-kimpanel)
- [Niri](https://github.com/niri-wm/niri)

## Development

See the GitHub issues for the research tasks and implementation milestones.
