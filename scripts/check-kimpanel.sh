#!/usr/bin/env bash
set -euo pipefail

printf '%s\n' '--- service ownership ---'
busctl --user status org.kde.impanel 2>&1 || true

printf '%s\n' '--- service tree ---'
busctl --user tree org.kde.impanel 2>&1 || true

printf '%s\n' '--- introspection ---'
busctl --user introspect org.kde.impanel /org/kde/impanel 2>&1 || true
