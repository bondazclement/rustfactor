#!/usr/bin/env bash
# Backtest du corpus complet PAR MORCEAUX (mémoire bornée).
# Chaque tranche est rejouée séparément, la table de calibration est
# chaînée (--calib-in/--calib-out) : apprentissage walk-forward honnête.
# Usage : backtest_corpus.sh [--gauss] [étiquette]
set -u
BASE="$(cd "$(dirname "$0")/.." && pwd)"
TMP="$BASE/analysis/tmp"
OUT="$BASE/analysis/out"
GAUSS=""
LABEL="student"
[ "${1:-}" = "--gauss" ] && { GAUSS="--gauss"; LABEL="gauss"; shift; }
LABEL="${1:-$LABEL}"
TABLE="$OUT/calib_${LABEL}.json"
LOG="$OUT/corpus_${LABEL}.log"
rm -f "$TABLE" "$LOG"

# Groupes = préfixe avant _journal ; tri par timestamp embarqué.
mapfile -t groups < <(ls "$TMP" | sed 's/_journal.*//' | sort -u | while read -r g; do
  ts=$(echo "$g" | grep -oE '[0-9]{8}T[0-9]{6}' | head -1)
  echo "${ts:-00000000T000000} $g"
done | sort | awk '{print $2}')

echo "[corpus] ${#groups[@]} tranches, mode=$LABEL" | tee -a "$LOG"
i=0
for g in "${groups[@]}"; do
  i=$((i+1))
  # Fichiers du groupe : jamais les _light si les complets existent
  # (les light sont des sous-ensembles → doublons d'événements).
  mapfile -t files < <(ls "$TMP/${g}"_journal*.ndjson 2>/dev/null | grep -v "_light" | sort)
  if [ ${#files[@]} -eq 0 ]; then
    mapfile -t files < <(ls "$TMP/${g}"_journal*.ndjson 2>/dev/null | sort)
  fi
  [ ${#files[@]} -eq 0 ] && continue
  CAL_ARGS=()
  [ -f "$TABLE" ] && CAL_ARGS=(--calib-in "$TABLE")
  # ulimit : 9 Go d'espace d'adressage max — le chunk meurt proprement
  # au lieu d'étouffer la machine.
  ( ulimit -v 9000000
    # shellcheck disable=SC2086
    nice -n 10 "$BASE/target/release/pm-backtest" \
      --journal "${files[@]}" --no-maker --quiet $GAUSS ${EXTRA_ARGS:-} \
      "${CAL_ARGS[@]}" --calib-out "$TABLE" 2>>"$LOG"
  ) | sed "s/^/[$i\/${#groups[@]} $g] /" >> "$LOG"
done
echo "[corpus] terminé" >> "$LOG"

# Agrégation
python3 - "$LOG" <<'PYEOF'
import re, sys
pnl = ent = fen = 0.0
bs = bn = 0.0
for l in open(sys.argv[1]):
    m = re.search(r"PnL total = ([+-][0-9.]+) \$ \| fenêtres=(\d+) \| entrées taker=(\d+)", l)
    if m:
        pnl += float(m.group(1)); fen += int(m.group(2)); ent += int(m.group(3))
    m = re.search(r"Brier\(p calibré\) = ([0-9.]+) sur (\d+) échantillons", l)
    if m:
        bs += float(m.group(1)) * int(m.group(2)); bn += int(m.group(2))
print(f"═══ CORPUS COMPLET: PnL={pnl:+.2f}$  fenêtres={fen:.0f}  entrées={ent:.0f}  "
      f"Brier={bs/bn if bn else float('nan'):.4f} (n={bn:.0f}) ═══")
PYEOF
