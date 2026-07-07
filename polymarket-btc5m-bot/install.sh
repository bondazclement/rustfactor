#!/usr/bin/env bash
# ╔══════════════════════════════════════════════════════════════════════╗
# ║  Installateur & console du bot Polymarket btc-updown-5m               ║
# ║  Multi-distribution (Fedora · Ubuntu · Debian · Arch · openSUSE)      ║
# ║  Multilingue : Français (natif) · English · Deutsch                   ║
# ╚══════════════════════════════════════════════════════════════════════╝
# Interface de navigation : chaque écran revient au menu (retour toujours
# possible). N'installe rien hors du gestionnaire de paquets, de rustup et
# d'un lien ~/.local/bin/pm-ctl. Idempotent : relançable sans risque.
set -uo pipefail
BASE="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"

# ─── Couleurs & pictogrammes ─────────────────────────────────────────────
if [ -t 1 ]; then
  B=$'\033[1m'; DIM=$'\033[2m'; N=$'\033[0m'
  RED=$'\033[31m'; GRN=$'\033[32m'; YLW=$'\033[33m'; BLU=$'\033[34m'
  CYA=$'\033[36m'; MAG=$'\033[35m'; GRY=$'\033[90m'
else B=""; DIM=""; N=""; RED=""; GRN=""; YLW=""; BLU=""; CYA=""; MAG=""; GRY=""; fi
P_OK="${GRN}✔${N}"; P_KO="${RED}✘${N}"; P_INFO="${CYA}▸${N}"; P_WARN="${YLW}⚠${N}"

# ─── Traductions ─────────────────────────────────────────────────────────
LG="fr"
declare -A T
tr() { local k="$LG:$1"; printf '%s' "${T[$k]:-${T[fr:$1]:-$1}}"; }

T[fr:titre]="Bot Polymarket btc-updown-5m — installateur & console"
T[en:titre]="Polymarket btc-updown-5m bot — installer & console"
T[de:titre]="Polymarket btc-updown-5m Bot — Installer & Konsole"
T[fr:distro]="Distribution"; T[en:distro]="Distribution"; T[de:distro]="Distribution"
T[fr:retour]="Entrée pour revenir au menu…"; T[en:retour]="Enter to return to the menu…"; T[de:retour]="Enter, um zum Menü zurückzukehren…"
T[fr:choix]="Votre choix"; T[en:choix]="Your choice"; T[de:choix]="Ihre Wahl"
T[fr:invalide]="Choix invalide"; T[en:invalide]="Invalid choice"; T[de:invalide]="Ungültige Wahl"
T[fr:aurevoir]="À bientôt."; T[en:aurevoir]="Goodbye."; T[de:aurevoir]="Auf Wiedersehen."

T[fr:m_titre]="MENU PRINCIPAL"; T[en:m_titre]="MAIN MENU"; T[de:m_titre]="HAUPTMENÜ"
T[fr:m1]="Installer (dépendances · Rust · compilation · tests · pm-ctl)"
T[en:m1]="Install (dependencies · Rust · build · tests · pm-ctl)"
T[de:m1]="Installieren (Abhängigkeiten · Rust · Build · Tests · pm-ctl)"
T[fr:m2]="Vérifier (tests + connectivité Polymarket)"
T[en:m2]="Verify (tests + Polymarket connectivity)"
T[de:m2]="Prüfen (Tests + Polymarket-Konnektivität)"
T[fr:m3]="Diagnostic (santé des flux · ressources · disque)"
T[en:m3]="Diagnostics (feed health · resources · disk)"
T[de:m3]="Diagnose (Feed-Status · Ressourcen · Speicher)"
T[fr:m4]="Configurer (paramètres du bot)"; T[en:m4]="Configure (bot parameters)"; T[de:m4]="Konfigurieren (Bot-Parameter)"
T[fr:m5]="Dry run — paper trading (aucun ordre réel)"
T[en:m5]="Dry run — paper trading (no real orders)"
T[de:m5]="Dry-Run — Paper-Trading (keine echten Orders)"
T[fr:m6]="Tester le chemin d'ordres RÉEL (identifiants)"
T[en:m6]="Test the REAL order path (credentials)"
T[de:m6]="ECHTEN Order-Pfad testen (Zugangsdaten)"
T[fr:m7]="Micro-test RÉEL (≤ 5 \$/ordre)"; T[en:m7]="REAL micro-test (≤ \$5/order)"; T[de:m7]="ECHTER Mikro-Test (≤ 5 \$/Order)"
T[fr:m8]="Tableau de bord web (http://localhost:7777)"
T[en:m8]="Web dashboard (http://localhost:7777)"
T[de:m8]="Web-Dashboard (http://localhost:7777)"
T[fr:m9]="Arrêter tout processus du bot"; T[en:m9]="Stop any running bot process"; T[de:m9]="Alle laufenden Bot-Prozesse stoppen"
T[fr:m0]="Quitter"; T[en:m0]="Quit"; T[de:m0]="Beenden"

T[fr:i_dep]="Phase 1/4 — Dépendances système"; T[en:i_dep]="Step 1/4 — System dependencies"; T[de:i_dep]="Schritt 1/4 — Systemabhängigkeiten"
T[fr:i_rust]="Phase 2/4 — Rust (rustup, toolchain stable)"; T[en:i_rust]="Step 2/4 — Rust (rustup, stable toolchain)"; T[de:i_rust]="Schritt 2/4 — Rust (rustup, stable)"
T[fr:i_build]="Phase 3/4 — Compilation release (quelques minutes)"; T[en:i_build]="Step 3/4 — Release build (a few minutes)"; T[de:i_build]="Schritt 3/4 — Release-Build (einige Minuten)"
T[fr:i_cli]="Phase 4/4 — Tests & installation du CLI pm-ctl"; T[en:i_cli]="Step 4/4 — Tests & pm-ctl CLI install"; T[de:i_cli]="Schritt 4/4 — Tests & pm-ctl-CLI-Installation"
T[fr:i_fin]="Installation terminée."; T[en:i_fin]="Installation complete."; T[de:i_fin]="Installation abgeschlossen."
T[fr:i_deps_ok]="Dépendances installées"; T[en:i_deps_ok]="Dependencies installed"; T[de:i_deps_ok]="Abhängigkeiten installiert"
T[fr:i_manuel]="Gestionnaire de paquets non reconnu — installez à la main :"
T[en:i_manuel]="Package manager not recognized — install manually:"
T[de:i_manuel]="Paketmanager nicht erkannt — manuell installieren:"
T[fr:i_rust_ok]="Rust présent"; T[en:i_rust_ok]="Rust ready"; T[de:i_rust_ok]="Rust bereit"
T[fr:i_tests]="tests réussis"; T[en:i_tests]="tests passed"; T[de:i_tests]="Tests bestanden"
T[fr:i_path]="Ajoutez ~/.local/bin au PATH (une fois) :"; T[en:i_path]="Add ~/.local/bin to PATH (once):"; T[de:i_path]="~/.local/bin zum PATH hinzufügen (einmalig):"

T[fr:a_reel]="Vous allez saisir vos identifiants Polymarket. Ils ne sont JAMAIS stockés."
T[en:a_reel]="You are about to enter your Polymarket credentials. They are NEVER stored."
T[de:a_reel]="Sie geben gleich Ihre Polymarket-Zugangsdaten ein. Sie werden NIE gespeichert."
T[fr:a_dryconf]="Lancer un dry run (paper) ? Aucun ordre réel ne peut partir."
T[en:a_dryconf]="Start a dry run (paper)? No real order can be sent."
T[de:a_dryconf]="Dry-Run (Paper) starten? Es kann keine echte Order gesendet werden."
T[fr:a_confirmer]="Confirmer ? [o/N]"; T[en:a_confirmer]="Confirm? [y/N]"; T[de:a_confirmer]="Bestätigen? [j/N]"
T[fr:a_annule]="Annulé."; T[en:a_annule]="Cancelled."; T[de:a_annule]="Abgebrochen."
T[fr:a_dash_on]="Dashboard lancé → ouvrez http://localhost:7777"; T[en:a_dash_on]="Dashboard started → open http://localhost:7777"; T[de:a_dash_on]="Dashboard gestartet → http://localhost:7777 öffnen"
T[fr:a_stop]="Processus arrêtés."; T[en:a_stop]="Processes stopped."; T[de:a_stop]="Prozesse gestoppt."
T[fr:a_nobin]="Binaire absent — lancez d'abord l'installation (choix 1)."
T[en:a_nobin]="Binary missing — run the installation first (choice 1)."
T[de:a_nobin]="Binärdatei fehlt — zuerst Installation ausführen (Auswahl 1)."

# ─── Détection de la distribution ────────────────────────────────────────
PKG=""; PKG_NAME=""; INSTALL_CMD=""; DEPS=""
detect_distro() {
  local id="" like=""
  [ -r /etc/os-release ] && . /etc/os-release && id="${ID:-}" && like="${ID_LIKE:-}"
  case " $id $like " in
    *fedora*|*rhel*|*centos*)
      PKG="dnf"; PKG_NAME="Fedora/RHEL"
      DEPS="gcc git curl zstd openssl-devel pkgconf-pkg-config lsof chrony"
      INSTALL_CMD="sudo dnf install -y --skip-unavailable $DEPS" ;;
    *ubuntu*|*debian*|*mint*|*pop*)
      PKG="apt"; PKG_NAME="Ubuntu/Debian"
      DEPS="build-essential git curl zstd libssl-dev pkg-config lsof chrony ca-certificates"
      INSTALL_CMD="sudo apt-get update -qq && sudo apt-get install -y $DEPS" ;;
    *arch*|*manjaro*)
      PKG="pacman"; PKG_NAME="Arch"
      DEPS="base-devel git curl zstd openssl pkgconf lsof chrony"
      INSTALL_CMD="sudo pacman -Sy --needed --noconfirm $DEPS" ;;
    *suse*|*opensuse*)
      PKG="zypper"; PKG_NAME="openSUSE"
      DEPS="gcc git curl zstd libopenssl-devel pkg-config lsof chrony"
      INSTALL_CMD="sudo zypper install -y $DEPS" ;;
    *) PKG=""; PKG_NAME="${id:-inconnue}"; DEPS="gcc git curl zstd openssl pkg-config lsof" ;;
  esac
}

# ─── Châssis d'affichage ─────────────────────────────────────────────────
banniere() {
  clear
  echo "${MAG}${B}╔════════════════════════════════════════════════════════════════════╗${N}"
  printf "${MAG}${B}║${N}  ${B}%-66s${N}${MAG}${B}║${N}\n" "$(tr titre)"
  echo "${MAG}${B}╚════════════════════════════════════════════════════════════════════╝${N}"
  printf "  ${GRY}%s : ${N}%s   ${GRY}·   Rust : ${N}%s\n\n" "$(tr distro)" \
    "$PKG_NAME" "$(command -v cargo >/dev/null 2>&1 && rustc --version 2>/dev/null | cut -d' ' -f2 || echo '—')"
}
titre_phase() { echo; echo "${BLU}${B}══ $* ══${N}"; }
pause() { echo; read -rp "${GRY}$(tr retour)${N}" _; }
confirmer() { local r; read -rp "${YLW}$(tr a_confirmer)${N} " r; [[ "$r" =~ ^[oOyYjJ]$ ]]; }

# ─── Actions ─────────────────────────────────────────────────────────────
act_install() {
  banniere
  titre_phase "$(tr i_dep)"
  if [ -n "$PKG" ]; then
    echo "${P_INFO} $PKG : $DEPS"
    eval "$INSTALL_CMD" && echo "${P_OK} $(tr i_deps_ok)"
    sudo systemctl enable --now chronyd >/dev/null 2>&1 || true
  else
    echo "${P_WARN} $(tr i_manuel) $DEPS"
  fi

  titre_phase "$(tr i_rust)"
  if ! command -v cargo >/dev/null 2>&1; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
    # shellcheck disable=SC1091
    source "$HOME/.cargo/env"
  fi
  echo "${P_OK} $(tr i_rust_ok) : $(rustc --version)"

  titre_phase "$(tr i_build)"
  (cd "$BASE" && cargo build --release) && echo "${P_OK} target/release/{pm-bot, pm-backtest, pm-replay, pm-dash}"

  titre_phase "$(tr i_cli)"
  (cd "$BASE" && cargo test --workspace --quiet 2>&1 | grep -E "test result" | \
    awk -v m="$(tr i_tests)" '{s+=$4} END {printf "   %d %s\n", s, m}')
  mkdir -p "$HOME/.local/bin"
  ln -sf "$BASE/scripts/pm-ctl" "$HOME/.local/bin/pm-ctl"
  chmod +x "$BASE/scripts/"*.sh "$BASE/scripts/pm-ctl" 2>/dev/null || true
  echo "${P_OK} pm-ctl → ~/.local/bin/pm-ctl"
  case ":$PATH:" in *":$HOME/.local/bin:"*) ;; *)
    echo "${P_WARN} $(tr i_path) echo 'export PATH=\$HOME/.local/bin:\$PATH' >> ~/.bashrc" ;;
  esac
  echo; echo "${GRN}${B}$(tr i_fin)${N}"
  pause
}

act_verifier() {
  banniere; titre_phase "$(tr m2)"
  [ -x "$BASE/target/release/pm-bot" ] || { echo "${P_WARN} $(tr a_nobin)"; pause; return; }
  (cd "$BASE" && cargo test --workspace --quiet 2>&1 | grep -E "test result" | \
    awk -v m="$(tr i_tests)" '{s+=$4} END {printf "   %d %s\n", s, m}')
  "$BASE/scripts/pm-ctl" sante
  pause
}
act_diag() { banniere; titre_phase "$(tr m3)"; "$BASE/scripts/pm-ctl" sante; echo; "$BASE/scripts/pm-ctl" statut; pause; }
act_config() { banniere; "$BASE/scripts/pm-ctl" config; echo; echo "${P_INFO} pm-ctl config init | editer"; pause; }
act_dry() {
  banniere; titre_phase "$(tr m5)"; echo "${P_INFO} $(tr a_dryconf)"; echo
  if confirmer; then "$BASE/scripts/pm-ctl" demarrer; else echo "${P_INFO} $(tr a_annule)"; fi
  pause
}
act_ordres() { banniere; echo "${P_WARN} $(tr a_reel)"; echo; "$BASE/scripts/tester-ordres.sh"; pause; }
act_micro()  { banniere; echo "${P_WARN} $(tr a_reel)"; echo; "$BASE/scripts/micro-test.sh" lancer; pause; }
act_dash() {
  banniere; titre_phase "$(tr m8)"
  [ -x "$BASE/target/release/pm-dash" ] || { echo "${P_WARN} $(tr a_nobin)"; pause; return; }
  pkill -x pm-dash 2>/dev/null; sleep 1
  nohup "$BASE/target/release/pm-dash" "$BASE" >/tmp/pm-dash.log 2>&1 & disown
  sleep 2; echo "${P_OK} $(tr a_dash_on)"; pause
}
act_stop() {
  banniere
  "$BASE/scripts/pm-ctl" arreter 2>/dev/null || true
  "$BASE/scripts/micro-test.sh" arreter 2>/dev/null || true
  pkill -x pm-dash 2>/dev/null || true
  echo "${P_OK} $(tr a_stop)"; pause
}

# ─── Sélection de la langue ──────────────────────────────────────────────
choisir_langue() {
  clear
  echo "${MAG}${B}  Polymarket btc-updown-5m${N}"
  echo "  ${GRY}Langue · Language · Sprache${N}"
  echo "    ${B}1${N}  🇫🇷 Français"
  echo "    ${B}2${N}  🇬🇧 English"
  echo "    ${B}3${N}  🇩🇪 Deutsch"
  local r; read -rp "  ▸ [1] " r
  case "$r" in 2) LG=en;; 3) LG=de;; *) LG=fr;; esac
}

# ─── Menu principal ──────────────────────────────────────────────────────
menu() {
  while true; do
    banniere
    echo "${B}$(tr m_titre)${N}"
    echo "   ${GRN}1${N}  📦 $(tr m1)"
    echo "   ${GRN}2${N}  ✅ $(tr m2)"
    echo "   ${GRN}3${N}  🩺 $(tr m3)"
    echo "   ${GRN}4${N}  ⚙️  $(tr m4)"
    echo "   ${CYA}5${N}  📊 $(tr m5)"
    echo "   ${YLW}6${N}  🔌 $(tr m6)"
    echo "   ${YLW}7${N}  💵 $(tr m7)"
    echo "   ${CYA}8${N}  🖥️  $(tr m8)"
    echo "   ${GRY}9${N}  ⏹️  $(tr m9)"
    echo "   ${GRY}0${N}  🚪 $(tr m0)"
    echo
    local c; read -rp "${B}$(tr choix)${N} ▸ " c
    case "$c" in
      1) act_install;; 2) act_verifier;; 3) act_diag;; 4) act_config;;
      5) act_dry;; 6) act_ordres;; 7) act_micro;; 8) act_dash;; 9) act_stop;;
      0) echo "${GRY}$(tr aurevoir)${N}"; exit 0;;
      *) echo "${P_KO} $(tr invalide)"; sleep 1;;
    esac
  done
}

# ─── Point d'entrée ──────────────────────────────────────────────────────
# `./install.sh --auto` : installation directe non interactive (VPS/CI).
detect_distro
if [ "${1:-}" = "--auto" ] || [ ! -t 0 ]; then
  LG="${PM_LANG:-fr}"; act_install; exit 0
fi
choisir_langue
menu
