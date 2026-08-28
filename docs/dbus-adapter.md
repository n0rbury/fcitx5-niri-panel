# Kimpanel D-Bus adapter

This stage implements the **panel side** of the KDE Kimpanel D-Bus protocol.

Fcitx 5's Kimpanel UI addon is the client. It owns
`org.kde.kimpanel.inputmethod` and calls the panel at
`org.kde.impanel` / `org.kde.impanel2`.

The exact ABI confirmed on the target machine is:

```text
org.kde.impanel
    Configure()
    Exit()
    LookupTablePageDown()
    LookupTablePageUp()
    ReloadConfig()
    Restart()
    SelectCandidate(i)
    TriggerProperty(s)
    PanelCreated()

org.kde.impanel2
    SetLookupTable(asasasbbii)
    SetRelativeSpotRect(iiii)
    SetRelativeSpotRectV2(iiiid)
    SetSpotRect(iiii)
    PanelCreated2()
```

`SetLookupTable` is three string arrays, two booleans, and two integers.
Semantics verified against the Fcitx 5.1.19 Kimpanel addon
(src/ui/kimpanel/kimpanel.cpp):

```text
as     labels       - candidate labels (auto or custom); row 0 has an empty
                      label and holds the auxiliary-down line when present
as     texts        - candidate text, already including comments
                      (textWithComment); row 0 is the auxiliary-down text
as     attrs        - candidate attributes (always empty in 5.1.19)
b      hasPrev
b      hasNext
i      pos          - cursor index over the transmitted rows, -1 when the
                      table is empty, +1 offset when the aux row exists
i      layout       - CandidateLayoutHint: 0 NotSet, 1 Vertical, 2 Horizontal,
                      3 Table
```

An empty-array message (all three arrays empty, `false false -1 0`) clears the
table. The panel normalizes this by splitting the aux-down row into
`PanelState::aux_down`, shifting `selected` down by one, and keeping only real
candidates in `candidates`.

The daemon prints every relevant inbound method. This is intentional for the
prototype so protocol behavior can be compared directly with `busctl monitor`.
