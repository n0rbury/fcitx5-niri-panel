#!/usr/bin/env bash
set -euo pipefail

exec dbus-monitor --session \
  "type='signal',interface='org.kde.kimpanel.inputmethod'"
