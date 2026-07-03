#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOCAL_BIN="$HOME/.local/bin"
BIN_NAME="Rustector_btc_5mn"
BIN_PATH="$SCRIPT_DIR/target/release/$BIN_NAME"

echo "[INFO] Installing Rustector_btc_5mn..."

if ! command -v cargo >/dev/null 2>&1; then
  echo "[INFO] Rust not found, installing rustup..."
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
  source "$HOME/.cargo/env"
fi

if command -v rustup >/dev/null 2>&1; then
  rustup update stable --no-self-update || true
  rustup component add rustfmt clippy || true
fi

if command -v dnf >/dev/null 2>&1; then
  rpm -q ca-certificates >/dev/null 2>&1 || sudo dnf install -y ca-certificates
elif command -v apt-get >/dev/null 2>&1; then
  dpkg -l ca-certificates 2>/dev/null | grep -q "^ii" || sudo apt-get install -y ca-certificates
fi

cd "$SCRIPT_DIR"
cargo build --release

mkdir -p "$LOCAL_BIN"
ln -sf "$BIN_PATH" "$LOCAL_BIN/$BIN_NAME" || true

echo "[OK] Built: $BIN_PATH"
echo "[OK] Run: $BIN_PATH"
echo "[OK] Or (if PATH includes ~/.local/bin): $BIN_NAME"
