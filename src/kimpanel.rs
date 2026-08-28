use crate::model::{PanelState, Rect};
use anyhow::{Context, Result};
use std::sync::{Arc, RwLock};
use zbus::{connection::Builder, interface, Connection};

pub const SERVICE_NAME: &str = "org.kde.impanel";
pub const OBJECT_PATH: &str = "/org/kde/impanel";
pub const V1_INTERFACE: &str = "org.kde.impanel";
pub const V2_INTERFACE: &str = "org.kde.impanel2";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelCommand {
    SelectCandidate(i32),
    PageUp,
    PageDown,
    TriggerProperty(String),
    Exit,
    Restart,
    ReloadConfig,
    Configure,
}

#[derive(Clone, Default)]
pub struct StateStore {
    inner: Arc<RwLock<PanelState>>,
}

impl StateStore {
    pub fn snapshot(&self) -> PanelState {
        self.inner.read().expect("state lock poisoned").clone()
    }

    fn set_lookup_table(
        &self,
        labels: Vec<String>,
        texts: Vec<String>,
        attrs: Vec<String>,
        has_previous: bool,
        has_next: bool,
        selected: i32,
        layout: i32,
    ) -> zbus::fdo::Result<()> {
        let mut state = self.snapshot();
        state
            .set_lookup_table(
                labels,
                texts,
                attrs,
                has_previous,
                has_next,
                selected,
                layout,
            )
            .map_err(|e| zbus::fdo::Error::InvalidArgs(e.to_string()))?;
        *self.inner.write().expect("state lock poisoned") = state.clone();

        println!(
            "SetLookupTable aux_down={:?} candidates={:?} labels={:?} attrs={:?} prev={} next={} selected={} layout={}",
            state.aux_down,
            state.candidates.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(),
            state.candidates.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(),
            state.candidates.iter().map(|c| c.attr.as_str()).collect::<Vec<_>>(),
            state.has_previous,
            state.has_next,
            state.selected,
            layout,
        );
        Ok(())
    }

    fn set_spot(&self, rect: Rect, scale: Option<f64>) {
        let mut state = self.inner.write().expect("state lock poisoned");
        state.spot = Some(rect);
        state.scale = scale;
        match scale {
            Some(scale) => println!(
                "SetRelativeSpotRectV2 x={} y={} width={} height={} scale={}",
                rect.x, rect.y, rect.width, rect.height, scale
            ),
            None => println!(
                "SetSpotRect/SetRelativeSpotRect x={} y={} width={} height={}",
                rect.x, rect.y, rect.width, rect.height
            ),
        }
    }
}

#[derive(Clone, Default)]
struct ImpanelV1;

#[interface(name = "org.kde.impanel")]
impl ImpanelV1 {
    async fn configure(&self) {
        println!("Configure");
    }

    async fn exit(&self) {
        println!("Exit");
    }

    async fn lookup_table_page_down(&self) {
        println!("LookupTablePageDown");
    }

    async fn lookup_table_page_up(&self) {
        println!("LookupTablePageUp");
    }

    async fn reload_config(&self) {
        println!("ReloadConfig");
    }

    async fn restart(&self) {
        println!("Restart");
    }

    async fn select_candidate(&self, index: i32) {
        println!("SelectCandidate index={index}");
    }

    async fn trigger_property(&self, property: &str) {
        println!("TriggerProperty property={property:?}");
    }

    #[zbus(signal)]
    async fn panel_created(ctxt: &zbus::object_server::SignalContext<'_>) -> zbus::Result<()>;
}

#[derive(Clone)]
struct ImpanelV2 {
    store: StateStore,
}

#[interface(name = "org.kde.impanel2")]
impl ImpanelV2 {
    async fn set_lookup_table(
        &self,
        labels: Vec<String>,
        candidates: Vec<String>,
        attrs: Vec<String>,
        has_previous: bool,
        has_next: bool,
        selected: i32,
        layout: i32,
    ) -> zbus::fdo::Result<()> {
        self.store.set_lookup_table(
            labels,
            candidates,
            attrs,
            has_previous,
            has_next,
            selected,
            layout,
        )
    }

    async fn set_spot_rect(&self, x: i32, y: i32, width: i32, height: i32) {
        self.store.set_spot(Rect { x, y, width, height }, None);
    }

    async fn set_relative_spot_rect(&self, x: i32, y: i32, width: i32, height: i32) {
        self.store.set_spot(Rect { x, y, width, height }, None);
    }

    async fn set_relative_spot_rect_v2(
        &self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        scale: f64,
    ) {
        self.store
            .set_spot(Rect { x, y, width, height }, Some(scale));
    }

    #[zbus(signal)]
    async fn panel_created2(ctxt: &zbus::object_server::SignalContext<'_>) -> zbus::Result<()>;
}

pub async fn run_panel(store: StateStore, verbose: bool) -> Result<()> {
    let connection: Connection = Builder::session()
        .context("create session D-Bus connection builder")?
        .name(SERVICE_NAME)
        .context("request org.kde.impanel on the session bus")?
        .serve_at(OBJECT_PATH, ImpanelV1)?
        .serve_at(
            OBJECT_PATH,
            ImpanelV2 {
                store: store.clone(),
            },
        )?
        .build()
        .await
        .context("build Kimpanel D-Bus connection")?;

    let ctxt_v1 = zbus::object_server::SignalContext::new(&connection, OBJECT_PATH)?;
    let ctxt_v2 = zbus::object_server::SignalContext::new(&connection, OBJECT_PATH)?;

    // Announce panel availability after both interfaces have been exported.
    ImpanelV1::panel_created(&ctxt_v1).await?;
    ImpanelV2::panel_created2(&ctxt_v2).await?;

    eprintln!("PanelCreated");
    eprintln!("PanelCreated2");
    eprintln!("owning {SERVICE_NAME} at {OBJECT_PATH}");
    if verbose {
        eprintln!("waiting for Fcitx5 Kimpanel method calls...");
    }

    tokio::signal::ctrl_c()
        .await
        .context("wait for Ctrl-C")?;

    drop(connection);
    Ok(())
}
