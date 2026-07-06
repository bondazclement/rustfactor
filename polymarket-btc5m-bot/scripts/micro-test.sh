#!/usr/bin/env bash
# micro-test.sh — micro-test RÉEL de A à Z (compte ≤ ~25 $).
#
#   ./scripts/micro-test.sh lancer [--duree H]   demande les credentials puis
#                                                lance le bot RÉEL en fond
#   ./scripts/micro-test.sh arreter              arrêt propre (plus d'ordres ;
#                                                les positions se règlent seules)
#   ./scripts/micro-test.sh statut               raccourci pm-ctl statut
#
# Garde-fous actifs pendant le test (en plus de ceux de la stratégie) :
#   5 $ max par ordre (≈ 5 parts, le minimum d'échange — les gains non
#   réclamés restent bloqués : petit ordre = plus d'ordres possibles dans
#   la nuit) · 20 ordres max · ARRÊT DÉFINITIF à −20 $ · kill-switch ✗.
# Réclamation des gains : MANUELLE pour l'instant (bouton Claim de
# polymarket.com) — l'auto-redeem exige le relayer (SDK TS/Python
# uniquement) : chantier documenté dans docs/VISION.md.
set -euo pipefail
BASE="$(cd "$(dirname "$(readlink -f "$0")")/.." && pwd)"
PIDF="$BASE/data_v2/micro-test.pid"

cas="${1:-lancer}"

if [ "$cas" = "arreter" ]; then
  if [ -f "$PIDF" ] && kill -0 "$(cat "$PIDF")" 2>/dev/null; then
    kill -TERM "$(cat "$PIDF")" && sleep 2
    kill -0 "$(cat "$PIDF")" 2>/dev/null && kill -9 "$(cat "$PIDF")" || true
    rm -f "$PIDF"
    echo "✔ Micro-test arrêté. Aucun nouvel ordre ne partira."
    echo "  Les positions déjà prises se règlent d'elles-mêmes à la résolution"
    echo "  des fenêtres (gains crédités sur votre compte Polymarket)."
  else
    pkill -TERM -f "[p]m-bot --live" 2>/dev/null && echo "✔ Bot réel arrêté." || echo "▸ Aucun micro-test en cours."
    rm -f "$PIDF"
  fi
  exit 0
fi
if [ "$cas" = "statut" ]; then
  exec "$BASE/scripts/pm-ctl" statut
fi

# ── lancer ───────────────────────────────────────────────────────────────
DUREE_H=4
[ "${2:-}" = "--duree" ] && DUREE_H="${3:-4}"

echo "══ MICRO-TEST RÉEL — mode standard « trader humain » (70 $ / 120 s / 0,98) ══"
echo "Plafonds : 5 \$/ordre (minimum du marché) · 20 ordres · perte max 20 \$ (arrêt définitif)"
echo "Durée : ${DUREE_H} h (arrêt automatique aligné sur une fin de fenêtre)"
echo
pgrep -f "[p]m-bot" >/dev/null && { echo "✘ Un bot tourne déjà (pm-ctl arreter ou ./scripts/micro-test.sh arreter d'abord)"; exit 1; }

# 1. Credentials (jamais stockés — vivent dans l'environnement du processus)
if [ -z "${POLYMARKET_PRIVATE_KEY:-}" ]; then
  read -rsp "Clé privée du wallet (0x…, saisie masquée) : " POLYMARKET_PRIVATE_KEY; echo
  export POLYMARKET_PRIVATE_KEY
fi
read -rp  "Adresse funder [défaut: 0x8f770bAC16B72c2771322E412b377Bbf0Ad6844b] : " F
export POLYMARKET_FUNDER="${F:-0x8f770bAC16B72c2771322E412b377Bbf0Ad6844b}"
read -rp  "Type de signature [défaut: 1 (compte Google)] : " ST
export POLYMARKET_SIG_TYPE="${ST:-1}"

# 2. Binaire live à jour
BIN="$BASE/target/release/pm-bot"
echo "▸ Compilation (--features live)…"
(cd "$BASE" && cargo build --release --features live -p pm-bot -q)

# 3. Config micro : preset standard + sizing calé sur le compte réel.
#    config.toml (presets pm-dash) reste intact — le test a la sienne.
cat > "$BASE/config-micro.toml" <<'EOF'
# Micro-test réel — généré par scripts/micro-test.sh
[taker]
mode_valeur = false
dist_frontiere_usd = 70.0
tau_frontiere_s = 120.0
prix_max_frontiere = 0.98
marge_ev = 0.01
bankroll = 23.0
max_notional = 5.0
kelly_fraction = 1.0     # le plafond de 5 $ fait le travail à cette échelle
EOF

# 4. Lancement (fond + PID + arrêt automatique aligné fenêtre)
ts=$(date -u +%Y%m%dT%H%M%S)
OUT="$BASE/data_v2/run_live_${ts}"
mkdir -p "$OUT"
now_s=$(date +%s); fin=$(( ( (now_s + DUREE_H*3600) / 300 + 1) * 300 + 40 - now_s ))
export PM_LIVE_ARME=oui PM_CALIB_PATH="$BASE/data_v2/calibration.json"
export PM_MAX_ORDRE_USD=5.2 PM_MAX_ORDRES=20 PM_PERTE_MAX_USD=20
RUST_LOG=info nohup timeout "$fin" "$BIN" --live \
  --config "$BASE/config-micro.toml" --out "$OUT" > "$OUT/run.log" 2>&1 &
echo $! > "$PIDF"
disown
sleep 6
if grep -aq "MODE RÉEL ARMÉ" "$OUT/run.log"; then
  grep -a "MODE RÉEL ARMÉ" "$OUT/run.log" | sed 's/\x1b\[[0-9;]*m//g;s/.*WARN pm_bot: /✔ /'
  echo "✔ Micro-test lancé (pid $(cat "$PIDF"), arrêt auto dans ~${DUREE_H} h)"
  echo "  Suivi : pm-ctl statut · pm-ctl suivre · http://localhost:7777 (chip RÉEL ARMÉ)"
  echo "  Arrêt : ./scripts/micro-test.sh arreter"
else
  echo "✘ Démarrage réel refusé — dernières lignes :"
  tail -5 "$OUT/run.log" | sed 's/\x1b\[[0-9;]*m//g'
  rm -f "$PIDF"
  exit 1
fi
