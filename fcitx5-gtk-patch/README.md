# fcitx5-gtk: content-relative caret rect on Wayland (gtk4 + gtk3)

Fixes in the IM modules (`gtk4/fcitximcontext.cpp`, `gtk3/fcitximcontext.cpp`):
on Wayland, convert the caret rect reported to Fcitx from toplevel-*surface*
coordinates to window-*content* coordinates.

## The bug

`SetRelativeSpotRectV2` rects must be relative to the window *content*
origin — that is what every Kimpanel consumer anchors to (this panel adds
niri's tile position; the GNOME shell extension adds mutter's frame rect).
fcitx5-gtk4 instead builds the rect in toplevel-*surface* (buffer)
coordinates, which include the app's CSD shadow margin:

- main windows: `+ gtk_native_get_surface_transform(native)` — for a CSD
  window that is the shadow margin (GTK 4.22 `gtk/gtkwindow.c:2274`,
  extents from the theme's window `box-shadow`, floored at
  `RESIZE_HANDLE_SIZE`=12, zero when maximized/fullscreen);
- popovers (e.g. Nautilus F2 rename): the `GDK_IS_POPUP` loop adds
  `gdk_popup_get_position_*`, which for Wayland popups is
  `GDK_SURFACE->x/y` — the popup's offset from its **parent surface
  origin** (`gdkpopup-wayland.c:1448`), so the accumulation ends in
  toplevel-surface coordinates too.

GTK3 has the same defect through its parent walk
(`gdk_window_coords_to_parent` up the window chain): the walk ends in
coordinates relative to the toplevel **GdkWindow, whose origin is the
client's buffer origin — CSD shadow margins included** (the `.decoration`
shadow, or margins apps declare via `gdk_window_set_shadow_width`, e.g.
Firefox/Zen). Measured on a GTK 3.24 CSD window: buffer 1326x1450 vs
window geometry 1274x1398 (26px per side), and the fcitx-reported rect
tracked buffer coordinates exactly. GTK3 has no public getter for those
margins (`gdk_window_set_shadow_width` is set-only), so the gtk3 fix
instead subtracts the content origin by widget geometry: the titlebar
widget if the window has one, else the bin child — both are placed at the
content origin, and at (0,0) whenever the margin is gone (maximized).

**Known limitation (Firefox/Zen):** an app that declares its own shadows
via `gdk_window_set_shadow_width` keeps the im module in the dark — the
margins live in GDK-private state (`GdkWindowImplWayland.margin_*`,
setter-only), the GTK widget tree already sits at the content origin, so
the gtk3 probe reads (0,0) and the offset survives the patch (verified on
Zen: ~40px, identical before/after). Fixing those apps requires the app
itself to report content-relative rects — an upstream Firefox issue, not
an im-module one. (GNOME + kimpanel users hit the same Zen offset, since
mutter's frame_rect also excludes shadows.)

Consumers then add the content origin on top, the margin is counted twice,
and the candidate bar lands one margin below the caret. Observed: +28px in
GNOME TextEditor (Adwaita shadow), 12-14px in minimal GTK4 windows, ~31px
in the Nautilus rename popover. The bar also cannot flip correctly near the
screen bottom: flipping above the reported caret lands on the real caret's
line.

GTK's own built-in Wayland IM context performs the same surface-coordinate
conversion (`gtkimcontextwayland.c:419`) — correctly, because
`zwp_text_input_v3.set_cursor_rectangle` genuinely wants surface coords.
The D-Bus/Kimpanel path does not.

Verified empirically with a GTK oracle app that prints the caret location,
the widget→native translation and `gtk_native_get_surface_transform` from
inside the process: the fcitx-reported rect equals `content + transform`
for a normal window and `content + 0` for a maximized one (shadow
dropped); popover rects track the parent surface offset the same way.

GTK3 is not affected: its module walks `gdk_window_coords_to_parent` and
never adds the transform (confirmed by the same oracle for plain and
headerbar/CSD windows).

## The fix

**gtk4** — the `+frame` add is kept; after the popup loop the rect is
converted once, on Wayland only (X11 keeps surface coordinates — its path
below converts them to root coordinates):

```c
if (fcitxcontext->is_wayland && surface) {
    if (auto *root_native = gtk_native_get_for_surface(surface)) {
        double rootX = 0, rootY = 0;
        gtk_native_get_surface_transform(root_native, &rootX, &rootY);
        area.x -= rootX;
        area.y -= rootY;
    }
}
```

This covers both main windows and popovers uniformly (popover natives
carry no shadow margin of their own — their transform is the negated CSS
margin, ≈0).

**gtk3** — after the parent walk:

```c
GtkWidget *holder = NULL;
gdk_window_get_user_data(window, (gpointer *)&holder);
if (holder != NULL && GTK_IS_WINDOW(holder)) {
    GtkWidget *origin = gtk_window_get_titlebar(GTK_WINDOW(holder));
    if (origin == NULL) {
        origin = gtk_bin_get_child(GTK_BIN(holder));
    }
    gint ox = 0, oy = 0;
    if (origin != NULL && gtk_widget_get_mapped(origin) &&
        gtk_widget_translate_coordinates(origin, holder, 0, 0, &ox, &oy)) {
        area.x -= ox;
        area.y -= oy;
    }
}
```

Applies to fcitx5-gtk 5.1.6 (Ubuntu 26.04); the same code exists on master.

## Build and deploy (fury17)

```bash
sudo apt install build-essential cmake extra-cmake-modules \
    libgtk-3-dev libgtk-4-dev libxkbcommon-dev
sudo apt source fcitx5-gtk           # into a scratch dir
cd fcitx5-gtk-*/
patch -p1 < <repo>/fcitx5-gtk-patch/0001-wayland-report-window-content-relative-caret-rect.patch
patch -p1 < <repo>/fcitx5-gtk-patch/0002-gtk3-wayland-report-window-content-relative-caret-rect.patch
cmake -B build -DCMAKE_BUILD_TYPE=Release -DENABLE_GIR=OFF \
    -DENABLE_GTK2_IM_MODULE=OFF -DENABLE_GTK3_IM_MODULE=ON \
    -DENABLE_GTK4_IM_MODULE=ON
cmake --build build --target im-fcitx5-gtk4 im-fcitx5-gtk3 -j$(nproc)

# replace the system modules (dpkg restores the originals on package upgrade)
sudo cp /usr/lib/x86_64-linux-gnu/gtk-4.0/4.0.0/immodules/libim-fcitx5.so \
        /usr/lib/x86_64-linux-gnu/gtk-4.0/4.0.0/immodules/libim-fcitx5.so.orig
sudo cp build/gtk4/libim-fcitx5.so \
        /usr/lib/x86_64-linux-gnu/gtk-4.0/4.0.0/immodules/libim-fcitx5.so
sudo cp /usr/lib/x86_64-linux-gnu/gtk-3.0/3.0.0/immodules/im-fcitx5.so \
        /usr/lib/x86_64-linux-gnu/gtk-3.0/3.0.0/immodules/im-fcitx5.so.orig
sudo cp build/gtk3/im-fcitx5.so \
        /usr/lib/x86_64-linux-gnu/gtk-3.0/3.0.0/immodules/im-fcitx5.so
```

The module is dlopened per process: every GTK app must be fully restarted
to pick it up (running apps keep the old code mapped — and swapping the
file under a running app crashes it, so close apps before deploying). No
fcitx restart is needed — the module runs inside the app.

Revert: restore the `.orig` files (or `sudo apt install --reinstall
fcitx5-frontend-gtk3 fcitx5-frontend-gtk4`).
