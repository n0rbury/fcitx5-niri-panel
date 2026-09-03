# fcitx5: anchor text-input-v3 popups with the compositor

One-condition change in the kimpanel UI addon (`src/ui/kimpanel/kimpanel.cpp`):
delegate `wayland_v2` (text-input-v3) input contexts to ClassicUI on any
compositor, not just KDE.

## Why

With a kimpanel panel active, the kimpanel addon is fcitx5's UI and relays
every input context's spot rect through the kimpanel D-Bus channel. A
kimpanel consumer anchors that rect to the window **content** origin (the
compositor's tile/frame position) — it cannot know an app's CSD shadow
margins, so apps whose surface origin differs from their content origin
(any GTK app with a CSD shadow; Firefox/Zen even declare their own margins
via `gdk_window_set_shadow_width`, invisible to both GTK and the
compositor) render the candidate window offset by those margins.

For a `wayland_v2` input context that relay is strictly worse than the
alternative: the context only exists where the compositor implements
input-method-v2, and there ClassicUI's input popup is anchored by the
**compositor itself** (`zwp_input_method_v2.getInputPopupSurface`) to the
exact text-input cursor — no rect relay, no margin problem. ClassicUI
bails out gracefully if the popup surface is unavailable.

Upstream gates this delegation behind `isKDE()` (Plasma's kimpanel widget
cannot position popups yet, so KDE delegates to ClassicUI). On
non-KDE compositors with input-method-v2 support (sway 1.10+, niri) the
same reasoning applies whenever a text-input-v3 app opts in — e.g. by
unsetting `GTK_IM_MODULE` for it per the fcitx wiki's Wayland
recommendations.

Applies to fcitx5 5.1.19 (Ubuntu 26.04).

## Build and deploy (fury17)

```bash
sudo apt install build-essential cmake extra-cmake-modules \
    nlohmann-json3-dev libwayland-dev libxkbcommon-dev \
    libdbus-1-dev libfmt-dev libevent-dev libuuid1-dev  # plus fcitx5's usual deps
sudo apt source fcitx5                    # into a scratch dir
cd fcitx5-*/
patch -p1 < <repo>/fcitx5-patch/0001-kimpanel-delegate-wayland-v2-to-classicui.patch
cmake -B build -DCMAKE_BUILD_TYPE=Release -DENABLE_X11=Off \
    -DENABLE_ENCHANT=Off -DENABLE_SPELL=Off -DENABLE_TEST=Off
cmake --build build --target kimpanel -j$(nproc)

# replace the addon inside the fcitx5 process (backup first)
sudo cp /usr/lib/x86_64-linux-gnu/fcitx5/libkimpanel.so \
        /usr/lib/x86_64-linux-gnu/fcitx5/libkimpanel.so.orig
sudo cp build/bin/libkimpanel.so \
        /usr/lib/x86_64-linux-gnu/fcitx5/libkimpanel.so
systemctl --user restart app-org.fcitx.Fcitx5@autostart.service
```

Only fcitx5 restarts — running applications are unaffected (unlike the GTK
im modules). Revert: restore the `.orig` file (or `sudo apt install
--reinstall fcitx5`) and restart the unit.

## Opting an app in

Unset `GTK_IM_MODULE` for that app's launch (e.g. a desktop-entry override
with `Exec=env -u GTK_IM_MODULE ...`); it then speaks text-input-v3 to the
compositor. The kimpanel panel stops rendering for it — fcitx5's
compositor-anchored ClassicUI popup takes over.
