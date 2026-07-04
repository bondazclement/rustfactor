# Roadmap

## Fait (validé en réel le 2026-07-04 — docs/VALIDATION_LIVE.md)

- ✅ Réseau ouvert + tunnel proxy CONNECT dans le client WS.
- ✅ Politique de strike tranchée et validée : LastAtOrBefore, 5/5 exact
  vs UI, 9/9 gelés à T0 confidence 1.0. Cadence mesurée : 1 tick/s,
  latence médiane ~250 ms.
- ✅ Issue estimée vs résolutions officielles : 6/6 (capture market_resolved
  via connexions CLOB chevauchantes, matching par winning_asset_id).
- ✅ PaperBroker : anti-répétition taker, fills maker sur trades réels,
  règlement + PnL par fenêtre.
- ✅ Corrections de terrain : abonnement chainlink sans filtre, ask maker
  aligné au tick, seuil z du taker proportionnel au temps restant.
- ✅ Archives de référence poussées dans data_samples/ (zstd).

## Étapes suivantes (ordre)

1. **Campagne paper longue** (heures) pour la calibration : distribution de
   z par horizon, autocorrélation des rendements Chainlink, coûts réels.
2. **Calibration du maker** — chantier prioritaire identifié : il porte de
   l'inventaire jusqu'au règlement (perd le côté faux) ; sorties plus
   agressives, pas de re-quote après stop, TP adaptatif.
3. Backtest complet dans pm-replay (rejouer les journaux data_samples →
   stratégies → fills simulés) pour itérer sans attendre le live.
4. Canal WSS **user** (fills/positions réels) pour préparer l'exécution.
5. Compression zstd intégrée au recorder (1,3 Go/h brut actuellement).
6. Feature `live` : LiveGateway (squelette documenté dans pm-execution),
   tests en très petites tailles, kill-switch. UNIQUEMENT après une
   campagne paper longue au PnL positif démontré.
7. Durcissement : archives legacy (data_low_latency, data_5m) toujours
   bienvenues pour étendre l'historique de non-régression ; gestion
   tick_size_change dans le maker ; persistance des états.
