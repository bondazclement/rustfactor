# data_samples — archives de référence (runs réels du 2026-07-04)

Journaux NDJSON v2 compressés (zstd) capturés en conditions réelles depuis
l'environnement Claude Code (proxy réseau, latence médiane ~250 ms). Ils
servent de **jeu de calibration et de non-régression** pour `pm-replay`.

Décompression : `zstd -d <fichier>.zst`
Analyse : `cargo run -p pm-replay -- cadence --journal <fichier.ndjson>`

## Contenu

| Archive | Période (UTC) | Fenêtres couvertes | Notes |
| --- | --- | --- | --- |
| `run_20260704T180831_*` | 18:08 → 18:35 | 1783188300 (partielle) → 1783189800 | Run v1 : 664 k trames, 560 Mo bruts. Taker sans anti-répétition (931 ordres identiques — bug corrigé ensuite) |
| `run_v2_20260704T183845_*` | 18:38 → 18:55 | 1783190100 (partielle) → 1783191000 | Run v2 : PaperBroker actif, 3 résolutions officielles capturées |
| `run_v3_20260704T185839_*` | 18:58 → 19:15 | 1783191300 (partielle) → 1783192200 | Run v3 : correctifs tick/threshold validés, confirmations ✓ automatiques |
| `run_v4_20260704T192609_*` | 19:26 → 19:59 | 1783193100 (partielle) → 1783194600 | Campagne v4 (6 fenêtres pleines) : 7/7 strikes, 6/6 confirmations ✓ ; fenêtre baissière 1783194600 = cas d'école des pertes maker (calibration) |

Chaque `_run.log` associé contient les logs applicatifs (strikes gelés,
décisions paper, règlements PnL, résolutions officielles).

## Vérité terrain (relevés manuels sur l'interface Polymarket)

« Price to beat » affichés par l'UI, à comparer au strike reconstruit
(politique `LastAtOrBefore`) — **5/5 exacts, écart 0,00 $** :

| Fenêtre | Affiché UI | Reconstruit |
| --- | --- | --- |
| btc-updown-5m-1783188600 | 63 146,29 | 63 146,2932 ✓ |
| btc-updown-5m-1783188900 | 63 267,87 | 63 267,8727 ✓ |
| btc-updown-5m-1783189200 | 63 206,89 | 63 206,8892 ✓ |
| btc-updown-5m-1783189500 | 63 121,94 | 63 121,9405 ✓ |
| btc-updown-5m-1783189800 | 63 126,93 | 63 126,9273 ✓ |

Résolutions officielles capturées (`market_resolved`) concordantes avec
l'issue estimée (tick final vs strike) : **12/12 sur v2+v3+v4**.
