use anyhow::Result;
use fcitx5_niri_panel::kimpanel::{run_panel, StateStore};

#[tokio::main]
async fn main() -> Result<()> {
    let verbose = std::env::args().any(|arg| arg == "--verbose" || arg == "-v");
    run_panel(StateStore::default(), verbose).await
}
