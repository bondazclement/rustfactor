# Roadmap

## Bloqué en attente d'éléments externes

1. **Pousser les archives dans le repo** (ou un échantillon) :
   - `data_low_latency (copie de log de "Rustector_btc_5mn_1")/window_1778341500/raw.ndjson`
     (≥ 1 000 premières lignes suffisent),
   - `Récuperation donnée polymarket (Copie) version avancée en python/data_5m/btc_1778343900.csv`.
   Si trop volumineux : `git lfs`, ou `head -n 5000 raw.ndjson > sample.ndjson`.
2. **Autoriser `*.polymarket.com`** dans la politique réseau de
   l'environnement Claude Code (ou exécuter sur machine locale).

## Étapes suivantes (ordre)

1. `pm-replay strike-validate` sur les fenêtres témoins → trancher la
   politique de strike (hypothèse actuelle : LastAtOrBefore) et mesurer la
   cadence/latence réelle des ticks Chainlink (`cadence`).
2. Run d'acquisition live de plusieurs heures (paper) → valider strike vs
   affichage UI sur ~50 fenêtres + vs `market_resolved` (sens).
3. Calibration des stratégies sur archives (distribution de z aux horizons
   5–300 s, autocorrélation des rendements Chainlink, coûts réels observés
   dans les trades du CLOB).
4. Backtest complet dans pm-replay (rejouer BusEvent → stratégies → fills
   simulés contre le carnet reconstruit).
5. Canal WSS **user** (fills/positions réels) + suivi d'inventaire maker
   (aujourd'hui : Inventory par défaut, fills non simulés en paper).
6. Simulateur de fills paper (croisement des quotes avec le carnet + trades).
7. Feature `live` : implémentation LiveGateway (squelette documenté dans
   pm-execution), tests en très petites tailles, kill-switch.
8. Durcissement : reconnexion CLOB sans trou (double connexion croisée au
   changement de fenêtre), gestion tick_size_change dans le maker,
   persistance des états de stratégie.
