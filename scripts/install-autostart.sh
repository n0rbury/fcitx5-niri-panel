#!/usr/bin/env bash
# Install the built panel binary and its systemd user service for autostart.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bin_src="$repo_root/target/release/fcitx5-niri-panel"

if [[ ! -x "$bin_src" ]]; then
    echo "release binary not found at $bin_src; building..." >&2
    (cd "$repo_root" && cargo build --release)
fi

install -Dm755 "$bin_src" "$HOME/.local/bin/fcitx5-niri-panel"
mkdir -p "$HOME/.config/systemd/user"
install -Dm644 "$repo_root/scripts/fcitx5-niri-panel.service" "$HOME/.config/systemd/user/fcitx5-niri-panel.service"

systemctl --user daemon-reload
systemctl --user enable --now fcitx5-niri-panel.service
systemctl --user restart fcitx5-niri-panel.service || true

echo "installed and started fcitx5-niri-panel (systemctl --user status fcitx5-niri-panel)"
