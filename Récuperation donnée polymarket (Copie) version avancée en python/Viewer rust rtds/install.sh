#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# polymarket-viewer — installateur one-command
# Usage: bash install.sh
# ─────────────────────────────────────────────────────────────────────────────

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

info()    { echo -e "${CYAN}[INFO]${NC}  $*"; }
ok()      { echo -e "${GREEN}[OK]${NC}    $*"; }
warn()    { echo -e "${YELLOW}[WARN]${NC}  $*"; }
error()   { echo -e "${RED}[ERROR]${NC} $*" >&2; exit 1; }

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo ""
echo -e "${CYAN}╔══════════════════════════════════════════╗${NC}"
echo -e "${CYAN}║   polymarket-viewer — installateur       ║${NC}"
echo -e "${CYAN}╚══════════════════════════════════════════╝${NC}"
echo ""

# ── 1. Rust / Cargo ──────────────────────────────────────────────────────────
info "Vérification de Rust..."
if command -v cargo &>/dev/null; then
    RUST_VER=$(rustc --version)
    ok "Rust déjà installé : $RUST_VER"
else
    warn "Rust introuvable. Installation via rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
    source "$HOME/.cargo/env"
    ok "Rust installé : $(rustc --version)"
fi

# ── 2. Toolchain stable à jour ───────────────────────────────────────────────
info "Mise à jour de la toolchain Rust stable..."
if command -v rustup &>/dev/null; then
    rustup update stable --no-self-update 2>&1 | tail -1
    ok "Toolchain à jour"
else
    warn "rustup non disponible, toolchain non mise à jour"
fi

# ── 3. Composants rustfmt / clippy (optionnels mais utiles) ─────────────────
info "Vérification des composants rust..."
if command -v rustup &>/dev/null; then
    rustup component add rustfmt clippy 2>/dev/null || true
    ok "Composants rustfmt et clippy prêts"
fi

# ── 4. Dépendances système ───────────────────────────────────────────────────
# Ce projet utilise 100% rustls (pas d'OpenSSL) — aucune lib C requise.
# Les seules dépendances sont: ca-certificates pour la validation TLS.
info "Vérification des certificats racine CA..."

if command -v dnf &>/dev/null; then
    # Fedora / RHEL
    if ! rpm -q ca-certificates &>/dev/null; then
        warn "Installation de ca-certificates..."
        sudo dnf install -y ca-certificates
    fi
    ok "ca-certificates présent (Fedora)"
elif command -v apt-get &>/dev/null; then
    # Debian / Ubuntu
    if ! dpkg -l ca-certificates 2>/dev/null | grep -q "^ii"; then
        warn "Installation de ca-certificates..."
        sudo apt-get install -y ca-certificates
    fi
    ok "ca-certificates présent (Debian/Ubuntu)"
elif command -v pacman &>/dev/null; then
    # Arch
    ok "Arch Linux — ca-certificates inclus dans ca-certificates package"
else
    warn "Gestionnaire de paquets inconnu. Vérifiez manuellement que ca-certificates est installé."
fi

# ── 5. Compilation release ───────────────────────────────────────────────────
info "Compilation en mode release (première fois ~2min, ensuite incrémentale)..."
cd "$SCRIPT_DIR"
cargo build --release 2>&1

if [ -f "$SCRIPT_DIR/target/release/polymarket-viewer" ]; then
    ok "Binaire compilé : target/release/polymarket-viewer"
    BIN_SIZE=$(du -sh "$SCRIPT_DIR/target/release/polymarket-viewer" | cut -f1)
    ok "Taille : $BIN_SIZE"
else
    error "La compilation a échoué. Consultez les erreurs ci-dessus."
fi

# ── 6. Lien symbolique optionnel dans ~/.local/bin ───────────────────────────
LOCAL_BIN="$HOME/.local/bin"
BIN_TARGET="$SCRIPT_DIR/target/release/polymarket-viewer"

if [[ ":$PATH:" == *":$LOCAL_BIN:"* ]]; then
    mkdir -p "$LOCAL_BIN"
    ln -sf "$BIN_TARGET" "$LOCAL_BIN/polymarket-viewer"
    ok "Lien symbolique créé : $LOCAL_BIN/polymarket-viewer"
    info "Vous pouvez lancer 'polymarket-viewer' depuis n'importe où."
else
    warn "~/.local/bin n'est pas dans votre PATH."
    warn "Ajoutez ceci dans ~/.bashrc ou ~/.zshrc :"
    warn "  export PATH=\"\$HOME/.local/bin:\$PATH\""
    warn "Puis relancez votre terminal et réexécutez install.sh."
fi

# ── 7. Résumé ────────────────────────────────────────────────────────────────
echo ""
echo -e "${GREEN}╔══════════════════════════════════════════╗${NC}"
echo -e "${GREEN}║         Installation terminée !          ║${NC}"
echo -e "${GREEN}╚══════════════════════════════════════════╝${NC}"
echo ""
echo -e "  Lancer :  ${CYAN}./target/release/polymarket-viewer${NC}"
echo -e "  Ou :      ${CYAN}cargo run --release${NC}"
echo -e "  Guide :   ${CYAN}cat GUIDE.md${NC}"
echo ""
