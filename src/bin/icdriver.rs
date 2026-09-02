//! Dev tool: drive a synthetic fcitx5 portal input context over D-Bus.
//!
//! Verifies Kimpanel routing without a GUI: with ClientSideInputPanel (bit 39)
//! unset, input-panel updates must reach org.kde.impanel (SetLookupTable);
//! with the bit set, updates go to UpdateClientSideUI. `--cursor=x,y` reports
//! a window-relative cursor rect via SetCursorRect, the way real portal
//! clients do, to exercise the kimpanel spot-rect path.

use std::time::Duration;
use zbus::proxy;

#[proxy(
    interface = "org.fcitx.Fcitx.InputMethod1",
    default_service = "org.fcitx.Fcitx5",
    default_path = "/org/freedesktop/portal/inputmethod"
)]
trait InputMethod1 {
    fn create_input_context(
        &self,
        args: Vec<(String, String)>,
    ) -> zbus::Result<(zbus::zvariant::OwnedObjectPath, Vec<u8>)>;
}

#[proxy(interface = "org.fcitx.Fcitx.InputContext1")]
trait InputContext1 {
    fn set_capability(&self, cap: u64) -> zbus::Result<()>;
    fn focus_in(&self) -> zbus::Result<()>;
    fn focus_out(&self) -> zbus::Result<()>;
    fn process_key_event(
        &self,
        keyval: u32,
        states: u32,
        flag: u32,
        is_release: bool,
        extra: u32,
    ) -> zbus::Result<bool>;
    #[zbus(name = "DestroyIC")]
    fn destroy_ic(&self) -> zbus::Result<()>;
    #[zbus(name = "SetCursorRect")]
    fn set_cursor_rect(&self, x: i32, y: i32, w: i32, h: i32) -> zbus::Result<()>;
}

#[tokio::main]
async fn main() -> zbus::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let client_side = args.iter().any(|a| a == "--client-side");
    let text = args
        .iter()
        .find_map(|a| a.strip_prefix("--text="))
        .unwrap_or("nihao")
        .to_string();
    // Seconds to keep composing before focus-out, leaving room for
    // external interaction (e.g. SelectCandidate calls) to be observed.
    let hold: u64 = args
        .iter()
        .find_map(|a| a.strip_prefix("--hold="))
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);
    // Window-relative cursor rect to report after focus-in, like a real
    // portal client would.
    let cursor: Option<(i32, i32)> = args.iter().find_map(|a| {
        let v = a.strip_prefix("--cursor=")?;
        let (x, y) = v.split_once(',')?;
        Some((x.parse().ok()?, y.parse().ok()?))
    });

    let conn = zbus::Connection::session().await?;
    let im = InputMethod1Proxy::new(&conn).await?;
    let (path, _client_id) = im.create_input_context(Vec::new()).await?;
    println!("created ic {path} client_side={client_side}");
    let ic = InputContext1Proxy::builder(&conn)
        .destination("org.fcitx.Fcitx5")?
        .path(path.clone())?
        .build()
        .await?;

    let mut cap: u64 = 0x52;
    if client_side {
        cap |= 1u64 << 39;
    }
    ic.set_capability(cap).await?;
    println!("capability 0x{cap:x}");
    ic.focus_in().await?;
    if let Some((x, y)) = cursor {
        ic.set_cursor_rect(x, y, 0, 24).await?;
        println!("cursor rect {x},{y}");
    }
    std::thread::sleep(Duration::from_millis(500));

    for ch in text.chars() {
        let keyval = ch as u32;
        println!("key {ch} down");
        ic.process_key_event(keyval, 0, 0, false, 0).await?;
        ic.process_key_event(keyval, 0, 0, true, 0).await?;
        std::thread::sleep(Duration::from_millis(80));
    }

    std::thread::sleep(Duration::from_secs(hold));
    ic.focus_out().await?;
    ic.destroy_ic().await?;
    println!("done");
    Ok(())
}
