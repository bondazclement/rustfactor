#!/usr/bin/env bash
# Installateur one-line du bot Polymarket btc-updown-5m.
# Cible : Fedora (testé pour Fedora 43, x86_64/AMD). Idempotent : peut être
# relancé sans risque. N'installe RIEN en dehors de dnf/rustup/cargo et
# d'un lien ~/.local/bin/pm-ctl.
set -euo pipefail

info() { printf '\033[36m▸\033[0m %s\n' "$*"; }
ok()   { printf '\033[32m✔\033[0m %s\n' "$*"; }

# 0. Où sommes-nous ? (le script vit à la racine du projet)
BASE="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"
info "Projet : $BASE"

# 1. Dépendances système (Fedora).
if command -v dnf >/dev/null 2>&1; then
  info "Installation des dépendances système (dnf)…"
  sudo dnf install -y --skip-unavailable gcc git curl zstd openssl-devel pkgconf-pkg-config lsof chrony >/dev/null
  ok "Dépendances système installées"
  # Horloge fiable = latences mesurées fiables.
  sudo systemctl enable --now chronyd >/dev/null 2>&1 || true
else
  info "dnf absent (pas Fedora ?) — installez manuellement : gcc git curl zstd openssl-devel pkg-config"
fi

# 2. Rust (rustup, stable).
if ! command -v cargo >/dev/null 2>&1; then
  info "Installation de Rust (rustup, toolchain stable)…"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable >/dev/null
  # shellcheck disable=SC1091
  source "$HOME/.cargo/env"
  ok "Rust installé : $(rustc --version)"
else
  ok "Rust déjà présent : $(rustc --version)"
fi

# 3. Compilation release.
info "Compilation (release, quelques minutes au premier build)…"
(cd "$BASE" && cargo build --release)
ok "Binaires : target/release/{pm-bot, pm-backtest, pm-replay}"

# 4. Tests (aucun réseau requis).
info "Tests…"
(cd "$BASE" && cargo test --workspace --quiet 2>&1 | grep -E "test result" | awk '{s+=$4} END {print "   " s " tests réussis"}')

# 5. CLI pm-ctl dans le PATH.
mkdir -p "$HOME/.local/bin"
ln -sf "$BASE/scripts/pm-ctl" "$HOME/.local/bin/pm-ctl"
chmod +x "$BASE/scripts/pm-ctl" "$BASE/scripts/campaign.sh"
case ":$PATH:" in
  *":$HOME/.local/bin:"*) ;;
  *) info "Ajoutez ~/.local/bin au PATH (une fois) : echo 'export PATH=\$HOME/.local/bin:\$PATH' >> ~/.bashrc" ;;
esac
ok "CLI installé : pm-ctl (essayez : pm-ctl aide)"

echo
ok "Installation terminée."
cat <<'FIN'

  Prochaines étapes :
    pm-ctl sante        # vérifier la connectivité Polymarket
    pm-ctl demarrer     # lancer le dry run (boucle de campagnes)
    pm-ctl statut       # superviser

  Documentation complète : docs/INSTALLATION_FEDORA.md
FIN
