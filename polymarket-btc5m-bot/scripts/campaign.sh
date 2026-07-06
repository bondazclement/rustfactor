#!/usr/bin/env bash
# Boucle de campagnes paper : tranches de ~50 min, archivage + push après
# chaque tranche, arrêt quand TARGET_ENTRIES entrées taker sont atteintes
# (ou MAX_CYCLES tranches).
#
# Usage : scripts/campaign.sh [MAX_ENTRY] [TARGET_ENTRIES] [MAX_CYCLES] [SLICE_SECS]
set -u
cd "$(dirname "$0")/.."
BASE="$(pwd)"
MAX_ENTRY="${1:-0.75}"
TARGET_ENTRIES="${2:-20}"
MAX_CYCLES="${3:-30}"
SLICE_SECS="${4:-3000}"

STATE="$BASE/data_v2/campaign_state.txt"
SUMMARY="$BASE/data_samples_campaign/campaign_summary.log"
mkdir -p "$BASE/data_v2/archives" "$BASE/data_samples_campaign"
total_entries=$(cat "$STATE" 2>/dev/null || echo 0)

echo "[campagne] démarrage: max_entry=$MAX_ENTRY cible=$TARGET_ENTRIES entrées, deja=$total_entries" | tee -a "$SUMMARY"

for cycle in $(seq 1 "$MAX_CYCLES"); do
  if [ "$total_entries" -ge "$TARGET_ENTRIES" ]; then
    echo "[campagne] cible atteinte ($total_entries entrées) — arrêt" | tee -a "$SUMMARY"
    break
  fi
  ts=$(date -u +%Y%m%dT%H%M%S)
  RUN_DIR="$BASE/data_v2/camp_${ts}"
  mkdir -p "$RUN_DIR"
  echo "[campagne] cycle $cycle → $RUN_DIR (tranche ${SLICE_SECS}s)" | tee -a "$SUMMARY"
  CFG_ARGS=()
  [ -f "$BASE/config.toml" ] && CFG_ARGS=(--config "$BASE/config.toml")
  # Durée alignée sur une frontière de fenêtre 5 min + 40 s de grâce :
  # tuer le bot juste après un règlement, jamais avec une position ouverte
  # (leçon du 06/07 : trade du cycle 5 gagnant mais jamais comptabilisé).
  now_s=$(date +%s)
  end_s=$((now_s + SLICE_SECS))
  aligned=$(( ((end_s / 300) + 1) * 300 + 40 ))
  slice=$((aligned - now_s))
  RUST_LOG=info PM_CALIB_PATH="$BASE/data_v2/calibration.json" \
    timeout "$slice" "$BASE/target/release/pm-bot" \
    "${CFG_ARGS[@]}" --out "$RUN_DIR" --max-entry "$MAX_ENTRY" > "$RUN_DIR/run.log" 2>&1
  rc=$?

  entries=$(grep -ac "TAKER:" "$RUN_DIR/run.log" || true)
  confirms=$(grep -ac "CONFIRME" "$RUN_DIR/run.log" || true)
  contradictions=$(grep -ac "CONTREDIT" "$RUN_DIR/run.log" || true)
  pnl=$(grep -a "PnL cumulé" "$RUN_DIR/run.log" | tail -1 | sed 's/.*PnL cumulé=//' || echo "?")
  total_entries=$((total_entries + entries))
  echo "$total_entries" > "$STATE"
  echo "[campagne] cycle $cycle ($(basename "$RUN_DIR")) fini (rc=$rc): entrées=$entries (cumul=$total_entries) pnl_tranche=$pnl confirmations=$confirms contradictions=$contradictions" | tee -a "$SUMMARY"

  # Archive complète (locale) + archive légère (poussée sur GitHub).
  name="camp_${ts}"
  for f in "$RUN_DIR"/journal_*.ndjson; do
    [ -e "$f" ] || continue
    seg=$(basename "$f" .ndjson)
    zstd -9 -T0 -q "$f" -o "$BASE/data_v2/archives/${name}_${seg}.ndjson.zst"
    python3 - "$f" "$BASE/data_samples_campaign/${name}_${seg}_light.ndjson" <<'PYEOF'
import json, sys
src, dst = sys.argv[1], sys.argv[2]
keep_clob = ("last_trade_price", "market_resolved", "tick_size_change")
with open(src) as fi, open(dst, "w") as fo:
    for line in fi:
        try:
            fr = json.loads(line)
        except Exception:
            continue
        st = fr.get("stream")
        if st in ("rtds", "gamma") or (st == "clob" and any(k in fr.get("raw", "") for k in keep_clob)):
            fo.write(line)
PYEOF
    zstd -9 -T0 -q --rm "$BASE/data_samples_campaign/${name}_${seg}_light.ndjson"
    rm -f "$f"
  done
  cp "$RUN_DIR/run.log" "$BASE/data_samples_campaign/${name}_run.log"

  # Push (avec retries réseau).
  cd "$BASE/.."
  git add polymarket-btc5m-bot/data_samples_campaign polymarket-btc5m-bot/data_samples_campaign/campaign_summary.log 2>/dev/null
  git commit -q -m "Campagne auto ${name}: ${entries} entrée(s), cumul ${total_entries}, pnl tranche ${pnl}" 2>/dev/null
  for wait in 2 4 8 16; do
    git push -q origin claude/polymarket-btc-bot-refactor-ky56an && break
    sleep "$wait"
  done
  cd "$BASE"
done
echo "[campagne] terminé: cumul=$total_entries entrées" | tee -a "$SUMMARY"
