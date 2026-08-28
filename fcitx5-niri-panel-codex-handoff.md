# Handoff: fcitx5-niri-panel

## Resolved (2026-08-28)

The open routing question is answered and empirically verified on this machine
(fcitx5 5.1.19, Niri, XDG_CURRENT_DESKTOP=niri):

- Source truth: updateSingleComponent in src/lib/fcitx/userinterfacemanager.cpp
  sends the input panel either to updateClientSideUIImpl() (dbus frontend
  emits UpdateClientSideUI) when the IC has CapabilityFlag::ClientSideInputPanel
  (bit 39), or to the active UI addon (kimpanel -> SetLookupTable) otherwise.
  The flag survives for wayland:* displays because useClientSideUI()
  (dbusfrontend.cpp) is true on non-GNOME desktops; it is always stripped for
  x11:* displays and on GNOME. GTK/Qt/wayland-im frontends never set the flag.
- Controlled proof (no GUI): a synthetic portal IC with bit 39 unset made
  Rime's nihao progression reach the panel as filled SetLookupTable
  (neng/ne/na -> ni/mud/ni -> nihao/nihui/nihai); the same run with bit 39 set
  sent UpdateFormattedPreedit + UpdateClientSideUI with the same candidates to
  the IC owner and nothing to the panel. Rime itself is healthy; it only
  looks dead in portal apps because they own the client-side path.
- Real traffic observed live: X11 Feishu IC sends empty-table clears and
  SetSpotRect to the panel; Ghostty (portal, bit 39) feeds UpdateClientSideUI.
- Dev tool: cargo run --bin icdriver [--client-side] [--text=...] drives a
  synthetic IC for repeatable verification.
- Panel ABI fixed: SetLookupTable is three string arrays (labels, texts,
  attrs), verified against kimpanel.cpp; docs/dbus-adapter.md corrected.

Renderer milestone (2026-08-28, later): a bottom-anchored wlr-layer-shell bar
(smithay-client-toolkit + cosmic-text) paints preedit/aux/candidates with the
selected row highlighted; the panel now also subscribes to the
org.kde.kimpanel.inputmethod signals. Niri layer surfaces cannot follow the
caret, so the bar is screen-edge anchored by design.

Interaction milestone (2026-08-28, later): candidate selection works
end-to-end - clicking a row (or calling org.kde.impanel SelectCandidate)
emits the matching signal from the panel connection and Fcitx commits the
candidate (verified live: SelectCandidate(0) during a held "nihao"
composition made the table clear and the bar hide, i.e. a commit happened).
Paging/property signals are wired the same way.

Next logical steps: visual validation on Feishu/GTK apps, paging buttons on
the bar (has_previous/has_next already tracked), and stash/paging buttons.

## Context

Repository:

- https://github.com/n0rbury/fcitx5-niri-panel

Goal:

Build a clean standalone Fcitx 5 candidate/preedit panel for Niri, primarily to solve unreliable/missing IME candidate popups under Niri, especially for XWayland/Chromium/Electron/Feishu-class applications.

The original reference project is:

- https://github.com/wengxt/gnome-shell-extension-kimpanel

The intended high-level architecture is currently:

```text
Fcitx5
  |
  | KDE Kimpanel protocol
  v
fcitx5-niri-panel
  |
  | Wayland popup / layer-shell (eventual renderer)
  v
Niri
```

The important lesson from the investigation is that the GNOME extension is itself the **panel-side renderer/consumer**. It is not an Fcitx input-context observer.

---

## Current repository state

The initial prototype was created and pushed to `master`.

It contains a small Rust project with:

```text
Cargo.toml
src/
  lib.rs
  main.rs
  model.rs
  kimpanel.rs
tests/
  model.rs
README.md
LICENSE-MIT
```

The initial state-model tests passed:

```text
4 tests passed
```

The initial `cargo run` printed a hard-coded fake `PanelState`.

A second prototype archive was produced locally to experiment with the D-Bus panel side. The important version is the corrected **panel-side** implementation, not the first attempt.

The user has already pushed the initial prototype themselves, so do not assume the generated local archive is the exact current Git state. Inspect the repository first.

---

## Critical protocol correction

There was an early mistaken assumption that our program should subscribe to:

```text
org.kde.kimpanel.inputmethod
```

That is WRONG for the panel implementation.

The correct roles are:

```text
Fcitx5 Kimpanel addon
    owns: org.kde.kimpanel.inputmethod
    path: /kimpanel
    sends signals such as:
      RegisterProperties
      UpdateProperty
      Enable
      UpdateAux
      UpdatePreeditText
      ShowAux
      ShowPreedit
      ShowLookupTable
      ...

Panel implementation (GNOME extension / our program)
    owns: org.kde.impanel
    path: /org/kde/impanel
    exports:
      org.kde.impanel
      org.kde.impanel2
    receives calls such as:
      SetLookupTable
      SetSpotRect
      SetRelativeSpotRect
      SetRelativeSpotRectV2
```

This direction was empirically confirmed on the user's system.

---

## User's actual Fcitx 5 environment

Relevant versions/data from the user's machine:

- Fcitx 5.1.19
- Rime 5.1.13
- Niri on Wayland
- Fcitx KDE Input Method Panel addon is installed and enabled.
- `/usr/share/fcitx5/addon/kimpanel.conf` contains:

```ini
[Addon]
Name=KDE Input Method Panel
Type=SharedLibrary
Library=libkimpanel
Category=UI
Version=5.1.19
UIPriority=50
OnDemand=True
Configurable=True

[Addon/Dependencies]
0=dbus:5.1.19
1=core:5.1.19

[Addon/OptionalDependencies]
0=classicui:5.1.19
```

`fcitx5-diagnose` reports:

- KDE Input Method Panel 5.1.19 present
- 0 disabled addons
- all addon libraries found
- enabled UI addons include Classic UI, KDE Input Method Panel, DBus Virtual Keyboard

Bus names from the user's machine:

```text
org.fcitx.Fcitx5              -> :1.346   (fcitx5)
org.fcitx.Fcitx-0             -> :1.348
org.freedesktop.portal.Fcitx  -> :1.347
org.kde.kimpanel.inputmethod  -> :1.346   (fcitx5)
org.kde.impanel               -> :1.411   (fcitx5-niri-panel)
```

Thus the user's Fcitx Kimpanel addon is definitely running.

---

## Exact panel-side ABI observed on the user's machine

The user ran:

```bash
busctl --user introspect org.kde.impanel /org/kde/impanel
```

and got:

```text
org.kde.impanel                     interface -          -            -
.Configure                          method    -          -            -
.Exit                               method    -          -            -
.LookupTablePageDown                method    -          -            -
.LookupTablePageUp                  method    -          -            -
.ReloadConfig                       method    -          -            -
.Restart                            method    -          -            -
.SelectCandidate                    method    i          -            -
.TriggerProperty                    method    s          -            -
.PanelCreated                       signal    -          -            -

org.kde.impanel2                    interface -          -            -
.SetLookupTable                     method    asasasbbii -            -
.SetRelativeSpotRect                method    iiii       -            -
.SetRelativeSpotRectV2              method    iiiid      -            -
.SetSpotRect                        method    iiii       -            -
.PanelCreated2                      signal    -          -            -
```

Important correction:

`asasasbbii` =

```text
array<string>
array<string>
array<string>
boolean
boolean
int32
int32
```

It is **three string arrays**, not four.

Do not guess the semantic names/order until verified against Fcitx source.

---

## Empirical end-to-end result

The corrected panel-side prototype successfully printed:

```text
PanelCreated
PanelCreated2
owning org.kde.impanel at /org/kde/impanel
```

Then Fcitx sent real calls to it:

```text
SetRelativeSpotRectV2 x=52 y=976 width=1 height=1 scale=1
SetRelativeSpotRectV2 x=212 y=976 width=1 height=1 scale=1
SetRelativeSpotRectV2 x=52 y=976 width=1 height=1 scale=1
SetRelativeSpotRectV2 x=62 y=976 width=1 height=1 scale=1
SetRelativeSpotRectV2 x=72 y=976 width=1 height=1 scale=1
SetRelativeSpotRectV2 x=82 y=976 width=1 height=1 scale=1
SetRelativeSpotRectV2 x=92 y=976 width=1 height=1 scale=1
SetRelativeSpotRectV2 x=102 y=976 width=1 height=1 scale=1
SetRelativeSpotRectV2 x=112 y=976 width=1 height=1 scale=1
```

This is a major success: Fcitx discovers the standalone panel and sends the cursor/spot rectangle to it. The x-coordinate tracks the caret as the user types.

A `busctl --user monitor` trace from the user's system also showed, for example:

```text
Sender=:1.346
Destination=org.kde.impanel
Path=/org/kde/impanel
Interface=org.kde.impanel2
Member=SetRelativeSpotRectV2

MESSAGE "iiiid" {
    INT32 112;
    INT32 976;
    INT32 1;
    INT32 1;
    DOUBLE 1;
};
```

---

## Why `SetLookupTable` has not appeared

This is the main unresolved question.

The user's focused input context from Fcitx DebugInfo is:

```text
IC [ba95ff6de32c4496a1488d325094545b]
program:
frontend:dbus
cap:c001000032
focus:1
```

`c001000032` includes the `ClientSideInputPanel` capability.

The user then captured the following from the generic D-Bus monitor:

```text
Type=signal
Sender=:1.346
Destination=:1.223
Path=/org/freedesktop/portal/inputcontext/18
Interface=org.fcitx.Fcitx.InputContext1
Member=UpdateClientSideUI
MESSAGE "a(si)ia(si)a(si)a(ss)iibb" {
    ARRAY "(si)" { };
    INT32 -1;
    ARRAY "(si)" { };
    ARRAY "(si)" { };
    ARRAY "(ss)" { };
    INT32 0;
    INT32 0;
    BOOLEAN false;
    BOOLEAN false;
};
```

and, for another key event, the same structure with empty arrays.

The client-side UI signature is:

```text
 a(si) i a(si) a(si) a(ss) i i b b
```

which corresponds to the client-side UI state (preedit, cursor position, aux data, candidates, candidate index, layout hint, prev/next flags).

Crucially, this signal is sent to the **owner of that specific input-context object**, not broadcast to arbitrary observers.

Therefore:

- We cannot simply subscribe to `UpdateClientSideUI` from a global daemon and get the current application's candidate state.
- Creating our own InputContext1 would create a separate input context; it would not spy on the existing application's context.

---

## Important observation about Kimpanel vs ClientSideInputPanel

The likely current application flow is:

```text
Application
   |
   | has ClientSideInputPanel capability
   v
Fcitx5
   |
   +---- UpdateClientSideUI ----> existing application/input-context owner
   |
   +---- SetRelativeSpotRectV2 -> org.kde.impanel
```

This explains the observed combination:

```text
UpdateClientSideUI      ✓
SetRelativeSpotRectV2   ✓
ShowLookupTable         ✓
SetLookupTable          ✗
```

The absence of `SetLookupTable` is therefore likely intentional for that input-context, rather than a failure of our D-Bus service.

However, this needs to be verified from Fcitx source before making a final architectural decision.

---

## Why the GNOME extension is relevant

The original GNOME Shell project is a real Kimpanel **panel implementation**. It:

1. owns `org.kde.impanel` / `org.kde.impanel2`;
2. emits `PanelCreated` / `PanelCreated2`;
3. receives Fcitx's panel method calls;
4. renders the input panel using GNOME Shell's St/Clutter/UI machinery.

That is why it can work well on GNOME.

Do not treat it as an InputContext1 observer.

The key design question is therefore not "how do we port the GNOME extension's Fcitx client?" but:

> Under what conditions does Fcitx route input-panel state to Kimpanel (`SetLookupTable`, etc.) versus the client-side UI (`UpdateClientSideUI`), and what does GNOME do that makes Kimpanel work reliably?

---

## Current hypotheses

### Hypothesis A: client-side UI is selected for the current application

The application's input context advertises `ClientSideInputPanel`, so Fcitx sends `UpdateClientSideUI` to it instead of sending the full lookup table through Kimpanel.

If true, our panel cannot obtain candidate data for such an input context through Kimpanel alone.

### Hypothesis B: Kimpanel still works for some other applications/contexts

The current tested context might be a special frontend path (portal/DBus/XWayland/client-side UI). GNOME may have applications/contexts where `ClientSideInputPanel` is not active and therefore receives `SetLookupTable` normally.

This needs a controlled comparison.

### Hypothesis C: the real Niri solution should operate at the Wayland IME-popup layer

For native Wayland applications, the compositor may be better positioned to control/redirect the native IME popup.

But this should NOT be chosen yet. First establish exactly why GNOME's Kimpanel path is selected and whether the standalone panel can reproduce that behavior.

---

## What NOT to do next

Do not:

- implement a fake global `InputContext1` observer and assume it will receive another app's candidate state;
- keep guessing D-Bus signatures;
- build the Wayland renderer before establishing how candidate state reaches the daemon;
- keep relying on `ShowLookupTable` as proof that `SetLookupTable` should follow;
- rip out Kimpanel just because the current tested context uses client-side UI.

---

## Recommended next investigation

### 1. Inspect Fcitx source around Kimpanel update routing

Use the Fcitx source currently installed/upstream to identify the exact branch deciding between:

```text
updateInputPanel() / SetLookupTable
```

and

```text
ClientSideInputPanel / UpdateClientSideUI
```

Specifically inspect the implementation of the Kimpanel addon and the input-context capability handling.

The question to answer precisely is:

```text
if ClientSideInputPanel is present:
    what exact path is taken?
else:
    what exact path is taken?
```

### 2. Compare a known GNOME Kimpanel setup

If possible, run the same Fcitx installation with the GNOME Shell Kimpanel extension or inspect its existing behavior/source.

Determine which input-context capabilities are present when `SetLookupTable` is actually seen by the GNOME extension.

### 3. Use multiple applications

Test the standalone panel with at least:

- GNOME Text Editor (GTK)
- Ghostty
- Zen Browser/Chromium-class app
- Feishu/XWayland if possible

For each, record:

```bash
busctl --user call org.fcitx.Fcitx5 /controller org.fcitx.Fcitx.Controller1 DebugInfo
```

and capture:

```bash
busctl --user monitor org.kde.impanel
```

Look for whether `SetLookupTable` appears for any input context.

### 4. Inspect `UpdateClientSideUI` only to understand the other path

Do not try to globally consume it. The purpose is to understand what Fcitx is doing and whether an application's UI is expected to own the candidate surface.

---

## Once `SetLookupTable` is confirmed

Implement the exact panel-side method:

```text
SetLookupTable(asasasbbii)
```

and verify the three string-array semantics from Fcitx source.

The normalized state should contain at least:

```rust
struct PanelState {
    candidates: Vec<Candidate>,
    selected: i32,
    has_previous: bool,
    has_next: bool,
    layout_hint: i32,
    preedit: ...,
    auxiliary: ...,
    spot: Option<Rect>,
}
```

Then the first useful headless demo is:

```text
Fcitx5
  -> SetLookupTable
  -> SetRelativeSpotRectV2
  -> fcitx5-niri-panel
  -> log normalized candidate state
```

Only after that should the Wayland renderer be introduced.

---

## Current known-good commands on the user's machine

Check Fcitx services:

```bash
busctl --user list | grep -i fcitx
```

Inspect panel API:

```bash
busctl --user introspect org.kde.impanel /org/kde/impanel
```

Monitor panel calls:

```bash
busctl --user monitor org.kde.impanel
```

Monitor all D-Bus messages if necessary:

```bash
busctl --user monitor
```

Get Fcitx debug info:

```bash
dbus-send --session \
  --print-reply \
  --dest=org.fcitx.Fcitx5 \
  /controller \
  org.fcitx.Fcitx.Controller1.DebugInfo
```

Restart Fcitx:

```bash
fcitx5-remote -r
```

---

## Current project status in one sentence

**We have successfully built and registered a standalone Kimpanel panel for Fcitx5 and verified that Fcitx sends it real `SetRelativeSpotRectV2` cursor positions; the remaining architectural question is why the tested input contexts receive candidate data through `UpdateClientSideUI` instead of `SetLookupTable`, and whether that differs from the GNOME Kimpanel path.**

---

## Suggested Codex starting prompt

Start by reading this handoff and the current repository. Then inspect the upstream Fcitx5 Kimpanel implementation and answer this exact question before modifying code:

> For Fcitx 5.1.19, what condition causes the Kimpanel addon to call `SetLookupTable` on `org.kde.impanel2` versus sending `UpdateClientSideUI` to an input context with `ClientSideInputPanel`? Reconstruct the exact control flow from source, and identify what GNOME Kimpanel changes (if anything) that makes its panel receive candidate tables. Then propose the smallest next experiment on Niri to distinguish the remaining hypotheses. Do not implement the Wayland renderer yet.
