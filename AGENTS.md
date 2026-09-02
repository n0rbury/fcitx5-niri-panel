# AGENTS.md

Guidance for working on this repository.

## What this is

A standalone Fcitx 5 Kimpanel-compatible candidate/preedit panel for Niri:
one wlr-layer-shell bar, rendered by this process, used as the single
candidate UI across Wayland-native and XWayland apps.

## Architecture (keep it this shape)

```text
Fcitx 5 Kimpanel UI
        |
        | org.kde.impanel / org.kde.impanel2  (single channel)
        v
fcitx5-niri-panel = PanelState -> themed Wayland renderer (layer shell)
```

- `src/kimpanel.rs` owns `org.kde.impanel` and is the only Fcitx-facing
  component. Everything downstream is rendering.
- There is exactly one renderer. Do not add a second one.
- The D-Bus ABI contract (signal names, argument order, quirks) is documented
  in `docs/dbus-adapter.md`; the user-facing mechanism story is in `README.md`.

## Design principles

1. **Kimpanel is the core interface.** It is the only panel-observable
   channel, and the only one that can ever cover XWayland/XIM clients: the
   compositor never sees an X11 caret, so compositor-anchored IME popups
   cannot serve XIM apps. Do not treat protocol differences between frontends
   as architecture; they are implementation details.

2. **Native Wayland positioning is not our problem.** Niri + Smithay already
   position `zwp_input_popup_surface_v2` popups for Fcitx's own ClassicUI.
   Our fallback for input contexts without an absolute caret rect is the
   per-session re-anchor: recreate the layer surface without an output
   binding at each hidden -> visible transition, and Niri maps it to its
   then-active output. No compositor IPC.

3. **Code is a liability.** Before adding a component, ask which
   user-visible behavior it enables and what would break without it. Delete
   code that exists because of a disproved assumption (precedent: the Niri
   IPC focus tracker and the UpdateClientSideUI adapter were both removed
   after evaluation). Do not keep speculative machinery "in case upstream
   changes".

4. **There is no client-side channel.** Fcitx's `UpdateClientSideUI` signal
   is unicast to the app's own bus connection; a third-party panel cannot
   observe it, and monitoring the bus invasively is wrong. Panel-side
   rendering for GTK/Qt/portal apps is achieved by Fcitx itself via the
   supported `useClientSideUI` mechanism
   (`XDG_CURRENT_DESKTOP=GNOME:niri` drop-in on the Fcitx5 autostart unit).
   Do not re-add an adapter for `UpdateClientSideUI`. Evidence and analysis:
   `docs/dbus-adapter.md`.

5. **Styling follows the GNOME Kimpanel extension principle.** Reuse the
   desktop shell's popup styling ideas; keep the panel's own theming minimal.
   Do not port Fcitx's ClassicUI theme machinery.

6. **No experimental protocols.** `xx-input-method-v2` /
   `xx-text-input-v3` are future upstream infrastructure; no compositor or
   IME implements them today. Watch upstream; never implement them here.

7. **No brittle assumption-stacking.** A single odd observation (e.g.
   portal apps reporting global-looking relative rects) is not a reason to
   add special-case code. Prefer the general mechanism and re-anchor
   behavior.

## Non-goals

- Compositor-specific IPC (Niri or otherwise).
- A second rendering path or theming engine.
- Observing or emulating the client-side UI channel.
- Implementing `xx-*` Wayland protocols.
- On-panel paging UI (paging signals exist; nothing invokes them yet) — only
  add if a real need shows up.

## Verification workflow

```bash
cargo test                                   # unit tests (model + D-Bus parsing)
cargo clippy --all-targets                   # must stay clean
```

Headless smoke test (no Niri needed):

```bash
./scripts/smoke.sh                           # private bus + SetLookupTable feed
# or manually: dbus-run-session + busctl call into org.kde.impanel
```

On the machine (fury17):

```bash
journalctl --user -u fcitx5-niri-panel -f
busctl --user monitor --match interface=org.fcitx.Fcitx.InputContext1  # raw IC traffic
niri msg outputs && niri msg layers
```

Deploy after release builds:

```bash
./scripts/install-autostart.sh               # build + install + systemd restart
```

If panel-side rendering ever regresses to in-app Fcitx windows, first check
the Fcitx5 override drop-in
(`~/.config/systemd/user/app-org.fcitx.Fcitx5@autostart.service.d/override.conf`)
is still present — it is required, not optional.
