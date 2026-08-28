use crate::model::{PanelState, Rect};
use anyhow::{Context, Result};
use serde::Serialize;
use std::sync::mpsc::Sender;
use std::sync::{Arc, RwLock};
use zbus::{connection::Builder, interface, Connection};

pub const SERVICE_NAME: &str = "org.kde.impanel";
pub const OBJECT_PATH: &str = "/org/kde/impanel";
pub const V1_INTERFACE: &str = "org.kde.impanel";
pub const V2_INTERFACE: &str = "org.kde.impanel2";

/// Emit an org.kde.impanel signal from this panel's connection (the sender
/// match that the fcitx5 kimpanel addon subscribes to). This is how panels
/// order candidate selection, paging, or property triggers.
pub async fn emit_to_fcitx<B: Serialize + zbus::zvariant::Type>(
    conn: &Connection,
    member: &str,
    body: &B,
) -> zbus::Result<()> {
    conn.emit_signal(None::<&str>, OBJECT_PATH, V1_INTERFACE, member, body).await
}

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

#[derive(Clone)]
pub struct StateStore {
    inner: Arc<RwLock<PanelState>>,
    notify: Option<Sender<()>>,
}

impl StateStore {
    pub fn new(notify: Option<Sender<()>>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(PanelState::default())),
            notify,
        }
    }

    fn mutate<F: FnOnce(&mut PanelState)>(&self, f: F) {
        f(&mut self.inner.write().expect("state lock poisoned"));
        if let Some(tx) = &self.notify {
            let _ = tx.send(());
        }
    }

    pub fn snapshot(&self) -> PanelState {
        self.inner.read().expect("state lock poisoned").clone()
    }

    pub fn set_preedit(&self, text: &str) {
        self.mutate(|s| {
            s.preedit = text.to_string();
            s.recompute_visible();
        });
    }

    pub fn set_preedit_visible(&self, visible: bool) {
        self.mutate(|s| {
            s.preedit_visible = visible;
            s.recompute_visible();
        });
    }

    pub fn set_preedit_cursor(&self, cursor: i32) {
        self.mutate(|s| s.preedit_cursor = cursor);
    }

    pub fn set_aux(&self, text: &str) {
        self.mutate(|s| {
            s.aux = text.to_string();
            s.recompute_visible();
        });
    }

    pub fn set_aux_visible(&self, visible: bool) {
        self.mutate(|s| {
            s.aux_visible = visible;
            s.recompute_visible();
        });
    }

    pub fn set_lookup_visible(&self, visible: bool) {
        self.mutate(|s| {
            if !visible {
                s.candidates.clear();
            }
            s.recompute_visible();
        });
    }

    pub fn set_lookup_cursor(&self, index: i32) {
        self.mutate(|s| s.selected = index);
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
        let mut state = self.inner.read().expect("state lock poisoned").clone();
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

        self.mutate(move |s| *s = state);
        let state = self.snapshot();
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
        self.mutate(move |s| {
            s.spot = Some(rect);
            s.scale = scale;
        });
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

/// Subscribe to the fcitx5 kimpanel addon state signals (/kimpanel,
/// org.kde.kimpanel.inputmethod) and feed them into the shared store.
///
/// Uses a dedicated connection so the match-rule stream never interferes with
/// the org.kde.impanel object server on the main connection.
pub async fn subscribe_input_method_signals(store: StateStore) -> Result<()> {
    use futures_util::StreamExt;
    use zbus::message::Type;
    use zbus::MatchRule;

    let conn = Connection::session().await?;
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .path("/kimpanel")?
        .interface("org.kde.kimpanel.inputmethod")?
        .build();
    let mut stream = zbus::MessageStream::for_match_rule(rule, &conn, Some(64)).await?;

    while let Some(msg) = stream.next().await {
        let Ok(msg) = msg else { continue };
        let header = msg.header();
        let Some(member) = header.member().map(|m| m.as_str()) else {
            continue;
        };
        match member {
            "UpdatePreeditText" => {
                if let Ok((text, _)) = msg.body().deserialize::<(String, String)>() {
                    store.set_preedit(&text);
                }
            }
            "ShowPreedit" => {
                if let Ok((visible,)) = msg.body().deserialize::<(bool,)>() {
                    store.set_preedit_visible(visible);
                }
            }
            "UpdatePreeditCaret" => {
                if let Ok((cursor,)) = msg.body().deserialize::<(i32,)>() {
                    store.set_preedit_cursor(cursor);
                }
            }
            "UpdateAux" => {
                if let Ok((text, _)) = msg.body().deserialize::<(String, String)>() {
                    store.set_aux(&text);
                }
            }
            "ShowAux" => {
                if let Ok((visible,)) = msg.body().deserialize::<(bool,)>() {
                    store.set_aux_visible(visible);
                }
            }
            "ShowLookupTable" => {
                if let Ok((visible,)) = msg.body().deserialize::<(bool,)>() {
                    store.set_lookup_visible(visible);
                }
            }
            "UpdateLookupTableCursor" => {
                if let Ok((index,)) = msg.body().deserialize::<(i32,)>() {
                    store.set_lookup_cursor(index);
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[derive(Clone)]
struct ImpanelV1 {
    conn: Connection,
}

#[interface(name = "org.kde.impanel")]
impl ImpanelV1 {
    async fn configure(&self) {
        println!("Configure");
        let _ = emit_to_fcitx(&self.conn, "Configure", &()).await;
    }

    async fn exit(&self) {
        println!("Exit");
        let _ = emit_to_fcitx(&self.conn, "Exit", &()).await;
    }

    async fn lookup_table_page_down(&self) {
        println!("LookupTablePageDown");
        let _ = emit_to_fcitx(&self.conn, "LookupTablePageDown", &()).await;
    }

    async fn lookup_table_page_up(&self) {
        println!("LookupTablePageUp");
        let _ = emit_to_fcitx(&self.conn, "LookupTablePageUp", &()).await;
    }

    async fn reload_config(&self) {
        println!("ReloadConfig");
        let _ = emit_to_fcitx(&self.conn, "ReloadConfig", &()).await;
    }

    async fn restart(&self) {
        println!("Restart");
        let _ = emit_to_fcitx(&self.conn, "Restart", &()).await;
    }

    async fn select_candidate(&self, index: i32) {
        println!("SelectCandidate index={index}");
        let _ = emit_to_fcitx(&self.conn, "SelectCandidate", &(index,)).await;
    }

    async fn trigger_property(&self, property: &str) {
        println!("TriggerProperty property={property:?}");
        let _ = emit_to_fcitx(&self.conn, "TriggerProperty", &(property,)).await;
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

pub async fn run_panel(store: StateStore, verbose: bool, render: bool) -> Result<()> {
    let mut store = store;
    let (notify_tx, notify_rx) = std::sync::mpsc::channel::<()>();
    store.notify = Some(notify_tx);

    let signal_store = store.clone();
    tokio::spawn(async move {
        if let Err(e) = subscribe_input_method_signals(signal_store).await {
            eprintln!("kimpanel signal subscription failed: {e}");
        }
    });

    let connection: Connection = Builder::session()
        .context("create session D-Bus connection builder")?
        .name(SERVICE_NAME)
        .context("request org.kde.impanel on the session bus")?
        .build()
        .await
        .context("build Kimpanel D-Bus connection")?;

    connection
        .object_server()
        .at(OBJECT_PATH, ImpanelV1 { conn: connection.clone() })
        .await
        .context("serve org.kde.impanel")?;
    connection
        .object_server()
        .at(
            OBJECT_PATH,
            ImpanelV2 {
                store: store.clone(),
            },
        )
        .await
        .context("serve org.kde.impanel2")?;

    if render {
        crate::render::spawn(store.clone(), verbose, notify_rx, connection.clone());
    }

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
