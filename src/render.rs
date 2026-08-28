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
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::reexports::client::globals::registry_queue_init;
use smithay_client_toolkit::reexports::client::protocol::{wl_output, wl_surface::WlSurface};
use smithay_client_toolkit::reexports::client::{backend::WaylandError, Connection, QueueHandle};
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

const BG_COLOR: [u8; 4] = [0x21, 0x21, 0x21, 0xe8]; // RGBA
const SEL_BG_COLOR: [u8; 4] = [0x2f, 0x5f, 0x8f, 0xff];
const TEXT_COLOR: TextColor = cosmic_text::Color(0xFF_F0_F0_F0);

struct Row {
    text: String,
    selected: bool,
}

fn build_rows(state: &PanelState) -> Vec<Row> {
    let mut rows = Vec::new();
    if state.preedit_visible && !state.preedit.is_empty() {
        rows.push(Row { text: state.preedit.clone(), selected: false });
    } else if state.aux_visible && !state.aux.is_empty() {
        rows.push(Row { text: state.aux.clone(), selected: false });
    }
    if !state.aux_down.is_empty() {
        rows.push(Row { text: state.aux_down.clone(), selected: false });
    }
    if !state.candidates.is_empty() {
        match state.layout {
            CandidateLayout::Horizontal => {
                let mut text = String::new();
                for (i, c) in state.candidates.iter().enumerate() {
                    if i > 0 {
                        text.push_str("   ");
                    }
                    text.push_str(&c.label);
                    text.push_str(&c.text);
                }
                rows.push(Row { text, selected: false });
            }
            _ => {
                for (i, c) in state.candidates.iter().enumerate() {
                    rows.push(Row {
                        text: format!("{}{}", c.label, c.text),
                        selected: i == state.selected as usize,
                    });
                }
            }
        }
    }
    rows
}

struct Panel {
    registry_state: RegistryState,
    output_state: OutputState,
    shm: Shm,
    layer: Option<LayerSurface>,
    pool: Option<SlotPool>,
    configured_size: (u32, u32),
    desired_height: u32,
    dirty: bool,
    font_system: FontSystem,
    swash: SwashCache,
    store: StateStore,
    rx: Receiver<()>,
    verbose: bool,
}

impl Panel {
    fn repaint(&mut self, qh: &QueueHandle<Self>) {
        let state = self.store.snapshot();
        let rows = build_rows(&state);
        let visible = state.visible && !rows.is_empty();
        let desired_height = if visible {
            rows.len() as u32 * LINE_HEIGHT + 2 * PADDING
        } else {
            0
        };

        let Some(layer) = self.layer.clone() else { return };
        if desired_height != self.desired_height {
            self.desired_height = desired_height;
            self.dirty = true;
            let width = self.configured_size.0.max(1);
            // A bottom-anchored layer may not request height 0; use height 1
            // with no attached buffer to stay invisible while hidden.
            layer.set_size(width, desired_height.max(1));
            layer.commit();
            if desired_height == 0 {
                return;
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
        let rows = build_rows(&state);
        if rows.is_empty() {
            return;
        }

        // BGRA pixel buffer (wl_shm ARGB8888 is little-endian BGRA).
        let mut px = vec![0u8; w as usize * height as usize * 4];
        for chunk in px.chunks_exact_mut(4) {
            chunk.copy_from_slice(&BG_COLOR);
        }
        let bgra = |r: u8, g: u8, b: u8, a: u8| [b, g, r, a];

        let mut text_buffer =
            TextBuffer::new(&mut self.font_system, Metrics::new(15.0, LINE_HEIGHT as f32));
        text_buffer.set_size(&mut self.font_system, Some(w as f32), Some(height as f32));
        let text = rows
            .iter()
            .map(|row| row.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        text_buffer.set_text(
            &mut self.font_system,
            &text,
            Attrs::new().color(TEXT_COLOR),
            Shaping::Advanced,
        );

        if let Some(sel) = rows.iter().position(|row| row.selected) {
            if let Some(run) = text_buffer.layout_runs().nth(sel) {
                let y = run.line_y.round() as i32;
                let row_h = run.line_height.round() as i32;
                for yy in y..(y + row_h) {
                    if yy < 0 || yy as u32 >= height {
                        continue;
                    }
                    for pixel in px
                        [(yy as usize) * w as usize * 4..(yy as usize + 1) * w as usize * 4]
                        .chunks_exact_mut(4)
                    {
                        pixel.copy_from_slice(&bgra(
                            SEL_BG_COLOR[0], SEL_BG_COLOR[1], SEL_BG_COLOR[2], SEL_BG_COLOR[3],
                        ));
                    }
                }
            }
        }

        text_buffer.draw(
            &mut self.font_system,
            &mut self.swash,
            TEXT_COLOR,
            |x, y, gw, gh, color| {
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
                        let out_a = a as u16 + (dst_a as u16 * (255 - a as u16)) / 255;
                        for (c, off) in [(r, 2usize), (g, 1), (b, 0)] {
                            px[idx + off] =
                                (c as u16 + (px[idx + off] as u16 * (255 - a as u16)) / 255)
                                    .min(255) as u8;
                        }
                        px[idx + 3] = out_a.min(255) as u8;
                    }
                }
            },
        );

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
                canvas.copy_from_slice(&px);
                let Some(layer) = self.layer.clone() else { return };
                layer.wl_surface().damage_buffer(0, 0, w as i32, height as i32);
                buffer.attach_to(layer.wl_surface()).expect("attach buffer");
                layer.commit();
                if self.verbose {
                    eprintln!("[render] bar w={w} h={height} rows={}", rows.len());
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
        qh: &QueueHandle<Self>,
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

pub fn spawn(store: StateStore, verbose: bool, rx: Receiver<()>) {
    std::thread::Builder::new()
        .name("renderer".into())
        .spawn(move || {
            if let Err(e) = run_renderer(&store, verbose, rx) {
                eprintln!("[render] disabled: {e}");
            }
        })
        .expect("spawn renderer thread");
}

fn run_renderer(store: &StateStore, verbose: bool, rx: Receiver<()>) -> anyhow::Result<()> {
    let conn = Connection::connect_to_env()?;
    let (globals, mut queue) = registry_queue_init::<Panel>(&conn)?;
    let qh = queue.handle();

    let registry_state = RegistryState::new(&globals);
    let output_state = OutputState::new(&globals, &qh);
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
    layer.set_anchor(Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
    layer.set_exclusive_zone(0);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    layer.set_size(0, 1);
    layer.commit();

    let mut panel = Panel {
        registry_state,
        output_state,
        shm,
        layer: Some(layer),
        pool: None,
        configured_size: (0, 0),
        desired_height: 0,
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
