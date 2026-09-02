//! Layer-shell candidate bar renderer.
//!
//! Runs on a dedicated thread with its own Wayland connection. It watches the
//! shared panel state through a notification channel and paints an opaque,
//! content-sized wlr-layer-shell bar with the preedit/aux line above the
//! candidates (one joined horizontal row by default, stacked rows for the
//! vertical layout), highlighting the selected candidate. When the state
//! carries an absolute spot rectangle the bar pins to that output and sits
//! just below the caret (flipping above it near the screen bottom); without
//! one it falls back to a centered bar at the bottom of the screen.

use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use cosmic_text::{
    Attrs, Buffer as TextBuffer, Color as TextColor, FontSystem, Metrics, Shaping, SwashCache,
};
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::output::OutputData;
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::reexports::client::globals::registry_queue_init;
use smithay_client_toolkit::reexports::client::protocol::{
    wl_output, wl_output::WlOutput, wl_pointer::WlPointer, wl_seat::WlSeat,
    wl_surface::WlSurface,
};
use smithay_client_toolkit::reexports::client::{
    backend::WaylandError, Connection, Proxy, QueueHandle,
};
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind, PointerHandler};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
    LayerSurfaceConfigure,
};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shm::slot::SlotPool;
use smithay_client_toolkit::shm::{Shm, ShmHandler};

use crate::kimpanel::StateStore;
use crate::model::{CandidateLayout, PanelState};

const LINE_HEIGHT: u32 = 22;
/// Horizontal inset. Zero: text and the full-row highlight run flush to the
/// panel edges, so no dark band shows on the left or right.
const PADDING: u32 = 0;
// Each row occupies exactly one 22px slot; a 15px font's baseline sits 14px
// below the slot top, which fills the slot with no clipping (CJK glyphs
// ascend ~16px and descend ~3px around that baseline).
/// Niri places a top-left anchored layer surface this many pixels below the
/// top margin (calibrated live on this machine; margin == surface top).
const FOLLOW_Y_OFFSET: i32 = -2;

// Dark skin (the approved look). All colors are straight RGBA; they are
// premultiplied into the ARGB8888 buffer while blending.
const BG_COLOR: [u8; 4] = [0x21, 0x21, 0x21, 0xff];
const BORDER_COLOR: [u8; 4] = [0x21, 0x21, 0x21, 0xff];
const SEL_BG_COLOR: [u8; 4] = [0x2f, 0x5f, 0x8f, 0xff];
const TEXT_COLOR: TextColor = cosmic_text::Color(0xFF_F0_F0_F0);
const SEL_TEXT_COLOR: TextColor = cosmic_text::Color(0xFF_F0_F0_F0);
const BAR_RADIUS: f32 = 0.0;
const PILL_RADIUS: f32 = 0.0;

struct Row {
    text: String,
    selected: bool,
    /// Visual index to report to fcitx on click (includes the aux-down row),
    /// set only for candidate rows.
    candidate: Option<usize>,
    /// True for the single joined row of a horizontally-laid-out candidate
    /// list; clicks on it must be resolved by x position.
    horizontal: bool,
}

/// Metrics of the laid-out text, taken from the actual shaped runs.
struct TextLayout {
    /// y of the first run's baseline, relative to the text buffer top.
    first_baseline: i32,
    /// y of the last run's baseline.
    last_baseline: i32,
    /// Widest run width.
    max_width: f32,
}

fn layout_metrics(buffer: &TextBuffer) -> TextLayout {
    let mut first_baseline = 0i32;
    let mut last_baseline = 0i32;
    let mut max_width = 0.0f32;
    let mut seen = false;
    for run in buffer.layout_runs() {
        if !seen {
            first_baseline = run.line_y.round() as i32;
            seen = true;
        }
        last_baseline = run.line_y.round() as i32;
        max_width = max_width.max(run.line_w);
    }
    if !seen {
        first_baseline = LINE_HEIGHT as i32 / 2 + 3;
        last_baseline = first_baseline;
    }
    TextLayout { first_baseline, last_baseline, max_width }
}

fn build_rows(state: &PanelState) -> Vec<Row> {
    let mut rows = Vec::new();
    if state.preedit_visible && !state.preedit.is_empty() {
        rows.push(Row { text: state.preedit.clone(), selected: false, candidate: None, horizontal: false });
    } else if state.aux_visible && !state.aux.is_empty() {
        rows.push(Row { text: state.aux.clone(), selected: false, candidate: None, horizontal: false });
    }
    if !state.aux_down.is_empty() {
        rows.push(Row { text: state.aux_down.clone(), selected: false, candidate: None, horizontal: false });
    }
    let aux_row = !state.aux_down.is_empty();
    if !state.candidates.is_empty() {
        match state.layout {
            CandidateLayout::Horizontal | CandidateLayout::NotSet => {
                let mut text = String::new();
                for (i, c) in state.candidates.iter().enumerate() {
                    if i > 0 {
                        text.push_str("   ");
                    }
                    text.push_str(&c.label);
                    text.push_str(&c.text);
                }
                rows.push(Row {
                    text,
                    selected: state.selected >= 0,
                    candidate: None,
                    horizontal: true,
                });
            }
            _ => {
                for (i, c) in state.candidates.iter().enumerate() {
                    rows.push(Row {
                        text: format!("{}{}", c.label, c.text),
                        selected: i == state.selected as usize,
                        candidate: Some(i + usize::from(aux_row)),
                        horizontal: false,
                    });
                }
            }
        }
    }
    rows
}

/// Width in pixels of text at the bar's font metrics, unwrapped.
fn text_width(font_system: &mut FontSystem, text: &str) -> f32 {
    let mut buffer =
        TextBuffer::new(font_system, Metrics::new(15.0, LINE_HEIGHT as f32));
    buffer.set_size(font_system, None, None);
    buffer.set_text(font_system, text, Attrs::new(), Shaping::Advanced);
    buffer
        .layout_runs()
        .next()
        .map(|run| run.line_w.ceil())
        .unwrap_or(0.0)
}

/// Screen-x ranges of each candidate in the horizontal row, matching the
/// exact strings painted by build_rows (label+text, three-space separators).
fn horizontal_segments(
    state: &PanelState,
    font_system: &mut FontSystem,
) -> Vec<(i32, i32)> {
    let mut segments = Vec::new();
    if state.candidates.is_empty() {
        return segments;
    }
    let sep = text_width(font_system, "   ") as i32;
    let mut x = 0i32;
    for candidate in &state.candidates {
        let text = format!("{}{}", candidate.label, candidate.text);
        let w = text_width(font_system, &text) as i32;
        segments.push((x, x + w));
        x += w + sep;
    }
    segments
}

/// Measure the natural content width of the bar for the given state.
pub fn estimate_bar_width(state: &PanelState, font_system: &mut FontSystem) -> u32 {
    let rows = build_rows(state);
    if rows.is_empty() {
        return 0;
    }
    let mut buffer =
        TextBuffer::new(font_system, Metrics::new(15.0, LINE_HEIGHT as f32));
    let text = rows
        .iter()
        .map(|row| row.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    buffer.set_size(font_system, None, None);
    buffer.set_text(font_system, &text, Attrs::new(), Shaping::Advanced);
    let metrics = layout_metrics(&buffer);
    let total = metrics.max_width.ceil() as u32 + 2 * PADDING;
    total.clamp(1, 2000)
}

/// Paint a filled rounded rectangle (straight-alpha RGBA color premultiplied
/// into the ARGB8888 buffer).
fn fill_round_rect(
    px: &mut [u8],
    w: u32,
    h: u32,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    radius: f32,
    color: [u8; 4],
) {
    let [r, g, b, a] = color;
    if a == 0 {
        return;
    }
    let aa = a as u32;
    let rad = radius.max(0.0);
    for yy in y0.max(0)..y1.min(h as i32) {
        for xx in x0.max(0)..x1.min(w as i32) {
            // Distance of the pixel center to the rounded-rect interior.
            let cx = xx as f32 + 0.5;
            let cy = yy as f32 + 0.5;
            let qx = (x0 as f32 + rad - cx).max(0.0);
            let qy = (y0 as f32 + rad - cy).max(0.0);
            let qx2 = (cx - (x1 as f32 - rad)).max(0.0);
            let qy2 = (cy - (y1 as f32 - rad)).max(0.0);
            let dx = qx.max(qx2);
            let dy = qy.max(qy2);
            let d = (dx * dx + dy * dy).sqrt();
            let cov = if d <= rad {
                255u32
            } else {
                // ~1px anti-aliased edge.
                (255.0 - (d - rad)).max(0.0).min(255.0) as u32
            };
            let idx = (yy as usize * w as usize + xx as usize) * 4;
            let src_a = (aa * cov / 255) as u16;
            let dst_a = px[idx + 3] as u16;
            let out_a = src_a + dst_a * (255 - src_a) / 255;
            for (c, off) in [(r, 2usize), (g, 1), (b, 0)] {
                px[idx + off] = ((c as u16 * src_a + px[idx + off] as u16 * (255 - src_a)) / 255)
                    .min(255) as u8;
            }
            px[idx + 3] = out_a.min(255) as u8;
        }
    }
}

/// Outline of a rounded rectangle (1px border), blending on top.
fn stroke_round_rect(
    px: &mut [u8],
    w: u32,
    h: u32,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    radius: f32,
    color: [u8; 4],
) {
    fill_round_rect(px, w, h, x0, y0, x1, y1, radius, color);
    fill_round_rect(
        px, w, h,
        x0 + 1, y0 + 1, x1 - 1, y1 - 1,
        (radius - 1.0).max(0.0),
        [0, 0, 0, 0],
    );
}

/// Blend one straight-alpha RGBA color over a pixel.
fn blend_px(px: &mut [u8], idx: usize, color: [u8; 4]) {
    let [r, g, b, a] = color;
    let src_a = a as u16;
    let dst_a = px[idx + 3] as u16;
    let out_a = src_a + dst_a * (255 - src_a) / 255;
    for (c, off) in [(r, 2usize), (g, 1), (b, 0)] {
        px[idx + off] =
            ((c as u16 * src_a + px[idx + off] as u16 * (255 - src_a)) / 255).min(255) as u8;
    }
    px[idx + 3] = out_a.min(255) as u8;
}

/// Paint the bar into a BGRA pixel buffer (wl_shm ARGB8888 order). Returns
/// None when there is nothing to show for the given state.
pub fn render_bar_pixels(
    state: &PanelState,
    w: u32,
    height: u32,
    font_system: &mut FontSystem,
    swash: &mut SwashCache,
) -> Option<Vec<u8>> {
    let rows = build_rows(state);
    if rows.is_empty() {
        return None;
    }

    let bgra = |r: u8, g: u8, b: u8, a: u8| [b, g, r, a];

    // Layout first: the bar is one exact 22px slot per row; each slot gets
    // the same baseline (14px down) so text and highlight geometry agree.
    let mut text_buffer =
        TextBuffer::new(font_system, Metrics::new(15.0, LINE_HEIGHT as f32));
    let text = rows
        .iter()
        .map(|row| row.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    text_buffer.set_size(font_system, Some(w as f32), Some(height as f32));
    text_buffer.set_text(
        font_system,
        &text,
        Attrs::new().color(TEXT_COLOR),
        Shaping::Advanced,
    );
    let layout = layout_metrics(&text_buffer);
    // Snap the first shaped baseline to 14px below the bar top; every row
    // then lands on its own slot grid (14, 36, 58, ...).
    let slot_baseline = LINE_HEIGHT as i32 / 2 + 5;
    let dy = slot_baseline - layout.first_baseline;
    // Horizontal text inset; bar width is content + 2 * PADDING.
    const TEXT_X: i32 = PADDING as i32;

    let mut px = vec![0u8; w as usize * height as usize * 4];

    // Background with rounded corners; alpha makes the corners transparent.
    fill_round_rect(
        &mut px, w, height,
        0, 0, w as i32, height as i32,
        BAR_RADIUS,
        BG_COLOR,
    );
    stroke_round_rect(
        &mut px, w, height,
        0, 0, w as i32, height as i32,
        BAR_RADIUS,
        BORDER_COLOR,
    );

    for (row_idx, run) in text_buffer.layout_runs().enumerate() {
        let row = &rows[row_idx];
        if !row.selected {
            continue;
        }
        let base_y = run.line_y.round() as i32;
        let slot_top = (row_idx as i32) * LINE_HEIGHT as i32;
        let x_range = if row.horizontal {
            let segs = horizontal_segments(state, font_system);
            let idx = state.selected.max(0) as usize;
            segs
                .get(idx)
                .map(|(a, b)| (a + TEXT_X, b + TEXT_X))
                .unwrap_or((TEXT_X, w as i32 - TEXT_X))
        } else {
            (TEXT_X - 2, w as i32 - TEXT_X + 2)
        };
        fill_round_rect(
            &mut px, w, height,
            x_range.0, slot_top, x_range.1, slot_top + LINE_HEIGHT as i32,
            PILL_RADIUS,
            SEL_BG_COLOR,
        );
    }

    // Draw text per row so the selected row can use a light color.
    for (row_idx, row) in rows.iter().enumerate() {
        let mut row_buffer =
            TextBuffer::new(font_system, Metrics::new(15.0, LINE_HEIGHT as f32));
        row_buffer.set_size(font_system, Some(w as f32), Some(LINE_HEIGHT as f32));
        let color = if row.selected {
            SEL_TEXT_COLOR
        } else {
            TEXT_COLOR
        };
        row_buffer.set_text(
            font_system,
            &row.text,
            Attrs::new().color(color),
            Shaping::Advanced,
        );
        let row_layout = layout_metrics(&row_buffer);
        // Align this row's baseline with the combined-layout baseline.
        let target_baseline = slot_baseline + (row_idx as i32) * LINE_HEIGHT as i32;
        let dy = target_baseline - row_layout.first_baseline;
        row_buffer.draw(font_system, swash, color, |x, y, gw, gh, gcolor| {
            let c = gcolor.0;
            let a = ((c >> 24) & 0xff) as u8;
            let r = ((c >> 16) & 0xff) as u8;
            let g = ((c >> 8) & 0xff) as u8;
            let b = (c & 0xff) as u8;
            for yy in (y + dy)..(y + dy + gh as i32) {
                if yy < 0 || yy as u32 >= height {
                    continue;
                }
                for xx in (x + TEXT_X)..(x + TEXT_X + gw as i32) {
                    if xx < 0 || xx as u32 >= w {
                        continue;
                    }
                    let idx = (yy as u32 * w + xx as u32) as usize * 4;
                    blend_px(&mut px, idx, bgra(r, g, b, a));
                }
            }
        });
    }

    Some(px)
}

struct Panel {
    registry_state: RegistryState,
    output_state: OutputState,
    seat_state: SeatState,
    dbus: zbus::Connection,
    rt: tokio::runtime::Runtime,
    compositor: CompositorState,
    layer_shell: LayerShell,
    shm: Shm,
    layer: Option<LayerSurface>,
    surface: Option<WlSurface>,
    layer_output: Option<WlOutput>,
    pool: Option<SlotPool>,
    configured_size: (u32, u32),
    desired_height: u32,
    desired_width: u32,
    layout: (Anchor, (i32, i32)),
    dirty: bool,
    font_system: FontSystem,
    swash: SwashCache,
    store: StateStore,
    rx: Receiver<()>,
    verbose: bool,
}

impl Panel {
    /// (logical_position, logical_size) of an output, if known.
    fn output_geometry(&self, output: &WlOutput) -> Option<((i32, i32), (i32, i32))> {
        output.data::<OutputData>().and_then(|data| {
            data.with_output_info(|info| match (info.logical_position, info.logical_size) {
                (Some(pos), Some(size)) => Some((pos, size)),
                _ => None,
            })
        })
    }

    /// The output whose logical rectangle contains the given point.
    fn output_containing(&self, x: i32, y: i32) -> Option<WlOutput> {
        self.output_state.outputs().find(|output| {
            self.output_geometry(output)
                .map(|((px, py), (pw, ph))| {
                    x >= px && x < px + pw && y >= py && y < py + ph
                })
                .unwrap_or(false)
        })
    }

    /// (Re)create the layer surface, optionally bound to a specific output.
    fn recreate_layer(&mut self, qh: &QueueHandle<Self>, output: Option<&WlOutput>) {
        if let Some(surface) = self.surface.take() {
            surface.destroy();
        }
        self.layer.take();
        let surface = self.compositor.create_surface(qh);
        let layer = self.layer_shell.create_layer_surface(
            qh,
            surface.clone(),
            Layer::Overlay,
            Some("fcitx5-niri-panel"),
            output,
        );
        layer.set_exclusive_zone(0);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_size(1, 1);
        self.surface = Some(surface);
        self.layer = Some(layer);
        self.layer_output = output.cloned();
        self.configured_size = (0, 0);
        self.desired_height = 0;
        self.desired_width = 0;
        self.dirty = true;
    }

    fn repaint(&mut self, qh: &QueueHandle<Self>) {
        let state = self.store.snapshot();
        let rows = build_rows(&state);
        let visible = state.visible && !rows.is_empty();
        let desired_height = if visible {
            // One exact 22px slot per row; the highlight fills the whole
            // selected slot so no background bands are possible.
            rows.len() as u32 * LINE_HEIGHT
        } else {
            0
        };
        let desired_width = estimate_bar_width(&state, &mut self.font_system).max(1);

        let Some(layer) = self.layer.clone() else { return };
        if desired_height != self.desired_height || desired_width != self.desired_width {
            self.desired_height = desired_height;
            self.desired_width = desired_width;
            self.dirty = true;
            // A bottom-anchored layer may not request height 0; use height 1
            // with no attached buffer to stay invisible while hidden.
            layer.set_size(desired_width, desired_height.max(1));
            layer.commit();
            if desired_height == 0 {
                return;
            }
        }

        // Follow the absolute spot rect (X11/XWayland path): the layer is
        // pinned to the output containing the caret and anchored top-left
        // with margins in that output's local coordinates. Otherwise, center
        // at the screen bottom (no output binding).
        // Only follow meaningful caret rectangles; fcitx also emits empty
        // (0,0,0,0) spots that must not yank the bar around.
        let follow = visible
            && state.spot_absolute
            && state
                .spot
                .map(|s| s.width > 0 || s.height > 0)
                .unwrap_or(false);
        let spot = follow.then(|| state.spot).flatten();
        let target_output = spot.and_then(|s| self.output_containing(s.x, s.y));
        // Keep one stable layer surface: bind to the spot's output when
        // following, otherwise keep the current binding (or the primary
        // output at startup). Never recreate on every state flip.
        let primary = self.output_state.outputs().next();
        let output = target_output
            .or_else(|| self.layer_output.clone())
            .or(primary);
        if output != self.layer_output {
            self.recreate_layer(qh, output.as_ref());
        }
        let layout = if let Some(spot) = spot {
            let (pos_x, pos_y) = output
                .as_ref()
                .and_then(|o| self.output_geometry(o))
                .map(|(p, _)| p)
                .unwrap_or((0, 0));
            let (_, (_, out_h)) = output
                .as_ref()
                .and_then(|o| self.output_geometry(o))
                .unwrap_or(((0, 0), (2560, 1440)));
            // Place the bar below the caret when there is room; flip above
            // the caret when typing near the bottom of the display.
            let gap = 4i32;
            let surface_top_below = spot.y + spot.height + gap;
            let surface_top = if surface_top_below + desired_height as i32 <= out_h {
                surface_top_below
            } else {
                (spot.y - desired_height as i32).max(0)
            };
            (
                Anchor::TOP | Anchor::LEFT,
                (
                    (spot.x - pos_x).max(0),
                    (surface_top - pos_y - FOLLOW_Y_OFFSET).max(0),
                ),
            )
        } else {
            (Anchor::BOTTOM, (0, 0))
        };
        if layout != self.layout {
            self.layout = layout;
            self.dirty = true;
            let (mleft, mtop) = layout.1;
            layer.set_anchor(layout.0);
            layer.set_margin(mtop, 0, 0, mleft);
            layer.commit();
            if self.verbose {
                eprintln!(
                    "[render] layout anchor={:?} margins=({mleft},{mtop}) output={:?}",
                    layout.0, self.layer_output
                );
            }
        }

        if !self.dirty {
            return;
        }
        self.dirty = false;
        self.draw(qh);
    }

    fn draw(&mut self, _qh: &QueueHandle<Self>) {
        if self.desired_height == 0 {
            return;
        }
        let (w, h) = self.configured_size;
        if w == 0 || h == 0 {
            return;
        }
        if h < self.desired_height {
            // Waiting for the configure that confirms the requested height.
            return;
        }
        let height = self.desired_height.min(h);
        let state = self.store.snapshot();
        let Some(px) = render_bar_pixels(
            &state,
            w,
            height,
            &mut self.font_system,
            &mut self.swash,
        ) else {
            return;
        };
        let pool = match self.pool.as_mut() {
            Some(pool) => pool,
            None => {
                let pool = SlotPool::new(w as usize * height as usize * 4, &self.shm)
                    .expect("create shm slot pool");
                self.pool.insert(pool)
            }
        };

        let stride = (w * 4) as i32;
        match pool.create_buffer(
            w as i32,
            height as i32,
            stride,
            smithay_client_toolkit::reexports::client::protocol::wl_shm::Format::Argb8888,
        ) {
            Ok((buffer, canvas)) => {
                // The reused slot may be larger than the current frame; only
                // the first w*h*4 bytes belong to this buffer.
                canvas[..px.len()].copy_from_slice(&px);
                let Some(layer) = self.layer.clone() else { return };
                layer.wl_surface().damage_buffer(0, 0, w as i32, height as i32);
                buffer.attach_to(layer.wl_surface()).expect("attach buffer");
                layer.commit();
                if self.verbose {
                    eprintln!("[render] bar w={w} h={height}");
                }
            }
            Err(e) => {
                if self.verbose {
                    eprintln!("[render] slot pool exhausted, skipping frame: {e}");
                }
            }
        }
    }
}

impl ProvidesRegistryState for Panel {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    smithay_client_toolkit::registry_handlers![OutputState];
}

impl OutputHandler for Panel {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
}

impl SeatHandler for Panel {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: WlSeat) {}

    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer {
            self.seat_state.get_pointer(qh, &seat).expect("create pointer");
        }
    }

    fn remove_capability(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _seat: WlSeat,
        _capability: Capability,
    ) {
    }

    fn remove_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: WlSeat) {}
}

impl PointerHandler for Panel {
    fn pointer_frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _pointer: &WlPointer,
        events: &[PointerEvent],
    ) {
        let Some(layer) = self.layer.clone() else { return };
        for event in events {
            if &event.surface != layer.wl_surface() {
                continue;
            }
            if let PointerEventKind::Press { button, .. } = event.kind {
                if button != 272 {
                    continue;
                }
                let state = self.store.snapshot();
                let x = event.position.0.max(0.0) as u32;
                let y = event.position.1.max(0.0) as u32;
                let rows = build_rows(&state);
                if rows.is_empty() {
                    continue;
                }
                let row = (y.saturating_sub(PADDING)) / LINE_HEIGHT;
                if let Some(r) = rows.get(row as usize) {
                    let candidate = if r.horizontal {
                        let segs = horizontal_segments(&state, &mut self.font_system);
                        segs.iter()
                            .position(|(s, e)| x >= *s as u32 && x < *e as u32)
                            .map(|i| i + usize::from(!state.aux_down.is_empty()))
                    } else {
                        r.candidate
                    };
                    if let Some(candidate) = candidate {
                        if self.verbose {
                            eprintln!("[render] click selects candidate {candidate}");
                        }
                        let dbus = self.dbus.clone();
                        let _ = self.rt.block_on(crate::kimpanel::emit_to_fcitx(
                            &dbus,
                            "SelectCandidate",
                            &(candidate as i32,),
                        ));
                    }
                }
            }
        }
    }
}

impl CompositorHandler for Panel {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _new_factor: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _time: u32,
    ) {
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl ShmHandler for Panel {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl LayerShellHandler for Panel {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _layer: &LayerSurface) {}

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        self.configured_size = configure.new_size;
        self.dirty = true;
    }
}

smithay_client_toolkit::delegate_registry!(Panel);
smithay_client_toolkit::delegate_compositor!(Panel);
smithay_client_toolkit::delegate_output!(Panel);
smithay_client_toolkit::delegate_shm!(Panel);
smithay_client_toolkit::delegate_layer!(Panel);
smithay_client_toolkit::delegate_seat!(Panel);
smithay_client_toolkit::delegate_pointer!(Panel);

pub fn spawn(store: StateStore, verbose: bool, rx: Receiver<()>, dbus: zbus::Connection) {
    std::thread::Builder::new()
        .name("renderer".into())
        .spawn(move || {
            if let Err(e) = run_renderer(&store, verbose, rx, dbus) {
                eprintln!("[render] disabled: {e}");
            }
        })
        .expect("spawn renderer thread");
}

fn run_renderer(
    store: &StateStore,
    verbose: bool,
    rx: Receiver<()>,
    dbus: zbus::Connection,
) -> anyhow::Result<()> {
    let conn = Connection::connect_to_env()?;
    let (globals, mut queue) = registry_queue_init::<Panel>(&conn)?;
    let qh = queue.handle();

    let registry_state = RegistryState::new(&globals);
    let output_state = OutputState::new(&globals, &qh);
    let seat_state = SeatState::new(&globals, &qh);
    let compositor = CompositorState::bind(&globals, &qh)?;
    let shm = Shm::bind(&globals, &qh)?;
    let layer_shell = LayerShell::bind(&globals, &qh)?;

    let surface = compositor.create_surface(&qh);
    let layer = layer_shell.create_layer_surface(
        &qh,
        surface.clone(),
        Layer::Overlay,
        Some("fcitx5-niri-panel"),
        None,
    );
    layer.set_exclusive_zone(0);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    layer.set_size(1, 1);
    layer.commit();

    let mut panel = Panel {
        registry_state,
        output_state,
        seat_state,
        dbus,
        compositor,
        layer_shell,
        rt: tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("create renderer runtime"),
        shm,
        layer: Some(layer),
        surface: Some(surface),
        layer_output: None,
        pool: None,
        configured_size: (0, 0),
        desired_height: 0,
        desired_width: 0,
        layout: (Anchor::BOTTOM, (0, 0)),
        dirty: true,
        font_system: FontSystem::new(),
        swash: SwashCache::new(),
        store: store.clone(),
        rx,
        verbose,
    };

    loop {
        // Non-blocking drain of the Wayland socket.
        if let Some(guard) = conn.prepare_read() {
            match guard.read() {
                Ok(_) | Err(WaylandError::Io(_)) => {}
                Err(e) => {
                    if verbose {
                        eprintln!("[render] read error: {e}");
                    }
                }
            }
        }
        if let Err(e) = queue.dispatch_pending(&mut panel) {
            if verbose {
                eprintln!("[render] dispatch error: {e}");
            }
        }
        while panel.rx.try_recv().is_ok() {
            panel.dirty = true;
        }
        let qh = queue.handle();
        // Coalesce bursts of D-Bus updates (fcitx sends preedit and lookup
        // table as separate messages): repaint at most every ~16ms so the
        // intermediate single-row frame is never shown.
        static LAST_PAINT: std::sync::Mutex<Option<Instant>> =
            std::sync::Mutex::new(None);
        let mut throttle = false;
        if panel.dirty {
            let mut last = LAST_PAINT.lock().unwrap();
            if let Some(t) = *last {
                if t.elapsed() < Duration::from_millis(16) {
                    throttle = true;
                }
            }
            if !throttle {
                *last = Some(Instant::now());
            }
        }
        if !throttle {
            panel.repaint(&qh);
        }
        let _ = conn.flush();
        std::thread::sleep(Duration::from_millis(8));
    }
}
