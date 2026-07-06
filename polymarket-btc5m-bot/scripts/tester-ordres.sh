#!/usr/bin/env bash
# tester-ordres.sh — test de A à Z du chemin d'ordres RÉEL Polymarket.
#
# Demande les credentials (jamais stockés, jamais affichés), découvre la
# fenêtre btc-updown-5m active, compile si besoin, puis exécute la batterie
# pm-live-test : auth → solde → ordre GTC 5 parts @ 0,01 $ (5 ¢ engagés,
# hors marché) → visible → annulation → carnet propre → latences.
set -euo pipefail
BASE="$(cd "$(dirname "$(readlink -f "$0")")/.." && pwd)"

echo "══ Test du chemin d'ordres réel — btc-updown-5m ══"
echo "Risque maximal de ce test : 5 ¢ immobilisés quelques secondes."
echo

# ── Credentials (à la volée, jamais écrits sur disque) ──────────────────
if [ -z "${POLYMARKET_PRIVATE_KEY:-}" ]; then
  read -rsp "Clé privée du wallet de TEST (0x…, saisie masquée) : " POLYMARKET_PRIVATE_KEY
  echo
  export POLYMARKET_PRIVATE_KEY
fi
read -rp  "Adresse funder (vide si wallet EOA lui-même) : " F
[ -n "$F" ] && export POLYMARKET_FUNDER="$F"
read -rp  "Type de signature [0=EOA, 1=Proxy, 2=Safe, 3=Poly1271] (défaut 0) : " ST
[ -n "$ST" ] && export POLYMARKET_SIG_TYPE="$ST"

# ── Fenêtre active → TOKEN_ID (token Up) ────────────────────────────────
echo "▸ Découverte de la fenêtre active…"
epoch=$(( $(date +%s) / 300 * 300 ))
TOKEN_ID=""
for off in 0 300; do
  slug="btc-updown-5m-$((epoch + off))"
  TOKEN_ID=$(curl -4 -sS --max-time 8 "https://gamma-api.polymarket.com/events?slug=$slug" \
    | python3 -c "import json,sys;e=json.load(sys.stdin);print(json.loads(e[0]['markets'][0]['clobTokenIds'])[0] if e else '')" 2>/dev/null) || true
  [ -n "$TOKEN_ID" ] && { echo "  fenêtre $slug, token Up ${TOKEN_ID:0:10}…"; break; }
done
[ -z "$TOKEN_ID" ] && { echo "✘ Impossible de trouver la fenêtre active"; exit 1; }
export TOKEN_ID

# ── Compilation si nécessaire, puis batterie ────────────────────────────
BIN="$BASE/target/release/pm-live-test"
if [ ! -x "$BIN" ]; then
  echo "▸ Compilation (--features live)…"
  (cd "$BASE" && cargo build --release --features live -p pm-execution)
fi
exec "$BIN"
