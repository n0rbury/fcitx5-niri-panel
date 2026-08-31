//! Layer-shell candidate bar renderer.
//!
//! Runs on a dedicated thread with its own Wayland connection. It watches the
//! shared panel state through a notification channel and paints a bottom
//! anchored wlr-layer-shell bar with preedit/aux/candidate rows, highlighting
//! the selected candidate. Niri anchors layer surfaces to screen edges, so the
//! bar does not follow the caret; it appears whenever input state is visible.

use std::sync::mpsc::Receiver;
use std::time::Duration;

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
const PADDING: u32 = 6;
/// Extra margin so the last glyph row is never clipped: cosmic-text lays out
/// the first line with a ~line-height/2 top offset.
const BOTTOM_MARGIN: u32 = 8;
/// Niri places a top-left anchored layer surface this many pixels below the
/// top margin (calibrated live on this machine; margin == surface top).
const FOLLOW_Y_OFFSET: i32 = -2;

const BG_COLOR: [u8; 4] = [0x21, 0x21, 0x21, 0xff]; // RGBA
const SEL_BG_COLOR: [u8; 4] = [0x2f, 0x5f, 0x8f, 0xff];
const TEXT_COLOR: TextColor = cosmic_text::Color(0xFF_F0_F0_F0);

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
    let mut max_line = 0.0f32;
    for run in buffer.layout_runs() {
        max_line = max_line.max(run.line_w);
    }
    let mut total = max_line.ceil() as u32 + 2 * PADDING;
    total.clamp(160, 2000)
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

    let mut px = vec![0u8; w as usize * height as usize * 4];
    for chunk in px.chunks_exact_mut(4) {
        chunk.copy_from_slice(&BG_COLOR);
    }
    let bgra = |r: u8, g: u8, b: u8, a: u8| [b, g, r, a];

    let mut text_buffer =
        TextBuffer::new(font_system, Metrics::new(15.0, LINE_HEIGHT as f32));
    text_buffer.set_size(font_system, Some(w as f32), Some(height as f32));
    let text = rows
        .iter()
        .map(|row| row.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    text_buffer.set_text(
        font_system,
        &text,
        Attrs::new().color(TEXT_COLOR),
        Shaping::Advanced,
    );

    if let Some(sel) = rows.iter().position(|row| row.selected) {
        if let Some(run) = text_buffer.layout_runs().nth(sel) {
            let base_y = run.line_y.round() as i32;
            // Glyph tops sit ~0.9 * font size above line_y; measured ink top
            // is line_y - 14 for our 15px font. Cover the full row slot.
            let row_top = base_y - 14;
            let x_range = if rows[sel].horizontal {
                let segs = horizontal_segments(state, font_system);
                let idx = state.selected.max(0) as usize;
                segs.get(idx).copied().unwrap_or((0, w as i32))
            } else {
                (0, w as i32)
            };
            for yy in row_top..(row_top + LINE_HEIGHT as i32) {
                if yy < 0 || yy as u32 >= height {
                    continue;
                }
                let row_start = (yy as usize) * w as usize * 4;
                let from = row_start + x_range.0.max(0) as usize * 4;
                let to = row_start + x_range.1.clamp(0, w as i32) as usize * 4;
                for pixel in px[from..to].chunks_exact_mut(4) {
                    pixel.copy_from_slice(&bgra(
                        SEL_BG_COLOR[0], SEL_BG_COLOR[1], SEL_BG_COLOR[2], SEL_BG_COLOR[3],
                    ));
                }
            }
        }
    }

    text_buffer.draw(font_system, swash, TEXT_COLOR, |x, y, gw, gh, color| {
        let c = color.0;
        let a = ((c >> 24) & 0xff) as u8;
        let r = ((c >> 16) & 0xff) as u8;
        let g = ((c >> 8) & 0xff) as u8;
        let b = (c & 0xff) as u8;
        for yy in y..(y + gh as i32) {
            if yy < 0 || yy as u32 >= height {
                continue;
            }
            for xx in x..(x + gw as i32) {
                if xx < 0 || xx as u32 >= w {
                    continue;
                }
                let idx = ((yy as u32) * w + xx as u32) as usize * 4;
                let dst_a = px[idx + 3];
                let ai = a as u16;
                let out_a = ai + (dst_a as u16 * (255 - ai)) / 255;
                for (c, off) in [(r, 2usize), (g, 1), (b, 0)] {
                    px[idx + off] =
                        ((c as u16 * ai + px[idx + off] as u16 * (255 - ai)) / 255).min(255)
                            as u8;
                }
                px[idx + 3] = out_a.min(255) as u8;
            }
        }
    });

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
            rows.len() as u32 * LINE_HEIGHT + 2 * PADDING + BOTTOM_MARGIN
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
        panel.repaint(&qh);
        let _ = conn.flush();
        std::thread::sleep(Duration::from_millis(30));
    }
}
