# Validation en conditions réelles — journal des runs (2026-07-04)

Tous les runs ont été exécutés en mode PAPER depuis l'environnement Claude
Code Remote (egress via proxy CONNECT, latence médiane ~250 ms incluse).
Archives brutes : `data_samples/` (zstd).

## Résultat central : le price to beat est résolu

**Politique validée : `strike = dernier update Chainlink avec
`payload.timestamp ≤ T0`** (`StrikePolicy::LastAtOrBefore`).

- **9/9 fenêtres** avec strike gelé en live à confidence 1.000, gap 0 ms ;
- **5/5 comparaisons** avec l'affichage de l'UI Polymarket : écart 0,00 $
  (relevés manuels, voir `data_samples/README.md`) ;
- le flux Chainlink émet 1 tick/s avec `payload.timestamp` à la seconde
  pile → il existe un tick exactement à T0 tant que l'acquisition est
  continue (c'est la raison d'être de la connexion RTDS permanente) ;
- politique alternative `FirstAtOrAfter` : fausse (−0,28 $ observé) ;
  interpolation legacy python : fausse (+0,06 $ historique).
- le champ `full_accuracy_value` (fixed-point 1e18) donne la précision
  exacte, archivé verbatim par le recorder.

## Issue de résolution : chaîne complète validée

Issue estimée = « dernier tick ≤ T_end > strike ? Up : Down », comparée
automatiquement à l'événement officiel `market_resolved` du canal CLOB :
**6/6 concordantes** (v2 : 3, v3 : 3). Particularité découverte : le `slug`
des `market_resolved` réels arrive vide → matching par `winning_asset_id`.

## Découvertes de terrain (écarts vs documentation)

1. **RTDS** : l'abonnement `crypto_prices_chainlink` avec filtre JSON
   documenté (`{"symbol":"btc/usd"}`) ne renvoie AUCUNE donnée. Il faut
   s'abonner sans filtre et filtrer côté client.
2. **RTDS** : le flux `crypto_prices` (btcusdt/Binance) n'émet rien avec le
   filtre documenté — non critique (affichage seulement), à re-tester.
3. **`market_resolved`** : slug vide (cf. ci-dessus) ; l'événement arrive
   ~90 s après la fin de fenêtre → les connexions CLOB doivent survivre à
   la rotation (implémenté : chevauchement jusqu'à résolution + 180 s max).
4. **Silences RTDS récurrents** : 3 épisodes de ~6 s en 75 min de runs.
   Le réabonnement forcé (>5,5 s) les récupère systématiquement ; le
   watchdog coupe la prise de risque pendant l'épisode.
5. **Volume** : ~1,3 Go/h de journal brut (dominé par les price_change
   CLOB) ; ratio zstd ≈ 12×.

## Chronologie des runs et corrections

| Run | Fenêtres | Constats | Corrections apportées |
| --- | --- | --- | --- |
| v1 (27 min) | 1783188300→1783189800 | Strike 5/5 exact ; taker répète la même décision toutes les 250 ms (931×) ; `market_resolved` jamais capturé (connexion coupée à la rotation) ; maker muet | PaperBroker (1 entrée/fenêtre/token, fills sur trades réels, PnL) ; connexions CLOB chevauchantes |
| v2 (16 min) | 1783190100→1783191000 | 3 entrées taker (fix ok) ; 3 résolutions capturées, 3/3 concordantes ; 1 412 re-quotes maker (ask non aligné au tick) ; slug vide → pas de confirmation auto ; entrée taker précoce perdante (z=2,6 à τ=290 s) ; PnL −113 $ | Ask aligné au tick supérieur ; matching par `winning_asset_id` ; seuil z ∝ temps restant (×2 pleine fenêtre → ×1 sous 60 s) + test de régression |
| v3 (16 min) | 1783191300→1783192200 | Re-quotes ÷5,5 (259) ; confirmations ✓ 3/3 ; 1 seule entrée taker, tardive, gagnante (+39,68 $) ; PnL −12,32 $ | — |
| v5 (33 min, 21:14→21:47) | 1783199400→1783201200 | Config calibrée (taker seul, maker off) : 7/7 strikes gelés à T0, 7/7 confirmations ✓, **0 entrée taker** (aucun signal z≥seuil avec prix ≤0.85 — marché en tendance douce), PnL 0,00 $. Comportement attendu : pas de trade sans edge | — |
| v4 (33 min) | 1783193100→1783194600 | Strike 7/7 gelés à T0 ; confirmations ✓ 6/6 ; 1 entrée taker (z=−3,5, gagnante) ; maker : +45 sur fenêtres calmes puis −147 sur la fenêtre baissière 1783194600 (37 fills, inventaire des DEUX côtés) ; PnL −179,71 $ | (analyse → calibration maker) |

## Lecture du PnL paper (cumul v2–v4)

Totaux : **strike gelé à T0 : 15/15 fenêtres pleines ; résolutions
officielles concordantes : 12/12 ; taker : 3 entrées, 2 gagnantes
(net ≈ +60 $) ; maker : centre de perte (≈ −250 $)**.

La fenêtre 1783194600 (v4) est le cas d'école de la faiblesse actuelle du
maker : marché baissier régulier, le fair bascule, le maker accumule de
l'inventaire sur les deux tokens (37 fills), les stops sortent trop tard et
le reliquat se règle à 0. Correctifs candidats (à backtester sur
data_samples avant activation) : n'être long que d'UN côté à la fois,
geler les entrées après un stop dans la même fenêtre, TP/stop asymétriques
selon τ, taille de quote décroissante avec l'inventaire.

## Lecture du PnL paper (v3 : −12,32 $ sur 4 fenêtres)

- **Taker : net positif** après le seuil temporel (1 trade, +39,68 $).
- **Maker : négatif** — cause identifiée dans les données : il porte de
  l'inventaire jusqu'au règlement (côté perdant −35/−39 $ par fenêtre alors
  que le côté gagnant fait +23/+26 $). Chantier de calibration prioritaire :
  sorties plus agressives (TP plus proche, stop plus serré, liquidation
  anticipée), et/ou ne pas re-quoter après un stop dans la même fenêtre.

## Backtest & calibration (2026-07-04, soirée)

Le backtest `pm-backtest` rejoue les 5 journaux (24 fenêtres, ~2,27 M
d'événements) avec exactement la même glue que le live (mêmes parseurs,
même PaperBroker, décisions au pas de 250 ms de temps journal).

Résultats :
- **Taker : positif sur les 135 configurations de la grille**
  (pire +31,73 $, meilleure +473,95 $). Zone robuste retenue comme défaut :
  `max_entry_price=0.85`, `min_abs_z=2.5` (×2 en début de fenêtre),
  quart de Kelly, edge ≥ 0.06. Sur cette config : +473,95 $ / 3 entrées,
  0 perdante. Leçon clé : le plafond de prix d'entrée à 0,85 élimine les
  pertes asymétriques (une entrée à 0,90 retournée coûtait −240 $).
- **Maker : négatif sur les 108 configurations de la grille**
  (meilleure −413 $), même avec inventaire mono-côté et gel après stop.
  Diagnostic : sélection adverse structurelle — les bids au repos sont
  remplis précisément quand le flux informé traverse. Ce n'est pas un
  problème de calibration mais de conception → **maker désactivé par
  défaut** dans pm-bot (`--maker` pour le réactiver en test), re-design
  nécessaire (idées : quoter uniquement très tôt dans la fenêtre, spread
  symétrique autour du fair avec sortie immédiate, ou market-making de
  la paire Up+Down ≈ 1).
- Interaction corrigée : les inventaires taker et maker sont désormais
  séparés dans le PaperBroker (le maker « gérait » les positions taker et
  détruisait leur espérance : −600 $ combiné vs +257 $ taker seul).

## Campagne « 20 entrées » (nuit du 04 au 05/07, max_entry=0.75)

- **Incident 22:20 (cycle 1)** : connexion RTDS à moitié morte (le tunnel
  accepte les écritures, ne livre plus rien) → 40 min de réabonnements
  inutiles. Garde-fous OK (0 trade), détecteur de contradictions OK (4 ✗
  signalées). Correctif : reconnexion complète forcée à 12 s de silence
  (RTDS) / 30 s (CLOB). Depuis : 14 confirmations, 0 contradiction.
- **Perte instructive 23:15 (−258 $)** : après un décrochage de ~220 $, le
  drift 120 s extrapolé sur les 289 s restantes a fabriqué z=−5,6 alors que
  l'écart au strike n'était que de 12 $ ; le prix a rebondi (issue Up).
  Correctif : **plafond de la contribution du drift à 2 unités de z**
  (`ProbConfig::max_drift_z`). Backtest sur les 55 fenêtres cumulées :
  cap=2.0 filtre cette entrée et domine l'absence de cap
  (0.75 : +329,83 $/3 entrées vs +252,55 $/4 ; 0.85 : +489,02 $/6 vs
  +466,47 $/8). Retirer le drift entièrement serait pire (+175,76 $) :
  il informe, il ne doit juste jamais dominer.
- Gagnante 00:00 : DOWN @0.741, z=−2,7, τ=76 s → +87,42 $.

## Prochaine étape

1. Poursuite de la boucle de campagnes (cible : 20 entrées cumulées) avec
   drift plafonné ; suivi via data_samples_campaign/campaign_summary.log.
2. Étoffer l'échantillon avant tout jugement définitif (régimes variés).
3. Re-design du maker, validé au backtest avant tout retour en live.
