#!/usr/bin/env bash
# Install the built panel binary and its systemd user service for autostart.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bin_src="$repo_root/target/release/fcitx5-niri-panel"

# Always build: cargo is incremental, so this is fast when up to date.
# (A mtime-based staleness check missed a source edit once and deployed
# a stale binary.)
(cd "$repo_root" && cargo build --release)

install -Dm755 "$bin_src" "$HOME/.local/bin/fcitx5-niri-panel"
mkdir -p "$HOME/.config/systemd/user"
install -Dm644 "$repo_root/scripts/fcitx5-niri-panel.service" "$HOME/.config/systemd/user/fcitx5-niri-panel.service"

systemctl --user daemon-reload
# Disable first: if the unit previously lived under default.target, a stale
# wants-symlink would start it before graphical-session.target exists.
systemctl --user disable fcitx5-niri-panel.service >/dev/null 2>&1 || true
systemctl --user enable --now fcitx5-niri-panel.service
systemctl --user restart fcitx5-niri-panel.service || true

echo "installed and started fcitx5-niri-panel (systemctl --user status fcitx5-niri-panel)"
