use anyhow::Result;
use fcitx5_niri_panel::kimpanel::{run_panel, StateStore};

#[tokio::main]
async fn main() -> Result<()> {
    let verbose = std::env::args().any(|arg| arg == "--verbose" || arg == "-v");
    let headless = std::env::args().any(|arg| arg == "--headless");
    run_panel(StateStore::new(None), verbose, !headless).await
}
