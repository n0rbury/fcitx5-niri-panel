use anyhow::Result;
use fcitx5_niri_panel::kimpanel::{update_lookup_table, StateStore};
use fcitx5_niri_panel::model::Rect;

fn main() -> Result<()> {
    let store = StateStore::default();

    // Temporary smoke-test harness. The real D-Bus service and Wayland renderer
    // will replace this in later commits.
    store.set_visible(true);
    store.set_preedit("ni");
    store.set_aux("");
    store.set_spot(Rect {
        x: 640,
        y: 360,
        width: 1,
        height: 24,
    });

    update_lookup_table(
        &store,
        &["1", "2", "3"],
        &["你", "呢", "泥"],
        &["", "", ""],
        0,
    )?;

    println!("{}", serde_json::to_string_pretty(&store.snapshot())?);
    Ok(())
}
