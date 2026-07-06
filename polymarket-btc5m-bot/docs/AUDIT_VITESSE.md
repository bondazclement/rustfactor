# Audit vitesse & stabilité — phase 2 (06/07/2026)

## Latences mesurées (timestamp source → notre horloge, journaux du soir)

| Flux | p50 | p90 | p99 | Verdict |
|---|---|---|---|---|
| Carnet CLOB (price_change) | **90 ms** | 244 ms | 2,2 s | excellent |
| Binance direct (btcusdt@trade) | **234 ms** | 365 ms | 592 ms | excellent |
| Spot via relais RTDS Polymarket | 467 ms | 613 ms | — | correct mais timestamps en retard de ~5 s |
| Oracle Chainlink via RTDS | **1 418 ms** | 1 814 ms | 2,2 s | incompressible (pipeline amont) |

Cadence oracle : 1 tick/s (p90 = 1 000 ms exactement).

## Correctif appliqué : décision ÉVÉNEMENTIELLE

Avant : boucle de décision sur minuterie 250 ms → jusqu'à 250 ms de retard
ajouté après chaque tick d'oracle. Après : évaluation immédiate à l'arrivée
de chaque tick (minuterie conservée en repli pour les mouvements de carnet
entre ticks). Gain moyen ~125 ms, pire cas 250 ms.

## Validation A/B (2 fenêtres, ancien et nouveau EN PARALLÈLE)

- Ticks oracle capturés : **733 vs 733, ensembles strictement identiques** ;
- Strikes gelés : identiques au dix-millième (63649.542285 / 63619.64156),
  confidence 1.000, gap 0 ms des deux côtés ;
- Règlements : 2 vs 2, mêmes issues.
→ Le correctif est exact ET plus rapide.

## Stabilité (correctifs déjà en place cette semaine)

- Reconnexion RTDS sur silence des VRAIS ticks (les PONG et le flux spot
  masquaient la mort du canal — incident du 06/07 17 h, corrigé) ;
- CLOB : silence mesuré sur les vraies trames ; reconnexion complète 30 s ;
- Boucle : tranches alignées sur les frontières de fenêtres (aucune
  position tuée avant règlement).

## Chemin d'ordres réel optimisé + batterie de tests A-Z

- Auth L1→L2 UNE fois au démarrage (client persistant, pas de
  re-négociation par ordre) ; signature EIP-712 locale ; le SDK
  auto-résout tick size/neg-risk/frais.
- **`scripts/tester-ordres.sh`** : demande les credentials (saisie masquée,
  jamais stockés), découvre la fenêtre active, puis batterie 7 étapes :
  auth → solde → ordre GTC 5 parts @ 0,01 $ (5 ¢ engagés, hors marché) →
  visibilité → annulation → carnet propre → rapport de latences.
  Risque maximal du test : 5 centimes immobilisés quelques secondes.
