#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VENV_PY="$ROOT_DIR/.venv/bin/python"

if [[ ! -x "$VENV_PY" ]]; then
  echo "Virtualenv not found. Run: python install.py wizard"
  exit 1
fi

cd "$ROOT_DIR"
exec "$VENV_PY" -m collector.cli run
