# Architecture

## Vue d'ensemble

```
                    ┌──────────────────────────────────────────────┐
                    │                 pm-acquisition               │
  RTDS (chainlink)──▶ rtds.rs ──┐                                  │
  RTDS (btcusdt)  ──▶           ├─▶ Recorder (NDJSON v2, verbatim) │
  CLOB market WS  ──▶ clob.rs ──┤                                  │
  Gamma REST      ──▶ gamma.rs ─┘        │ Bus (broadcast)         │
                    └────────────────────┼─────────────────────────┘
                                         ▼
                    ┌──────────────────────────────────────────────┐
                    │              pm-bot (Engine)                 │
                    │  ticks → VolEstimator (σ, μ)                 │
                    │  ticks → StrikeTracker (LastAtOrBefore, gel) │
                    │  clob  → OrderBook up / down                 │
                    │        → MarketSnapshot (250 ms + événement) │
                    └───────┬──────────────────────┬───────────────┘
                            ▼                      ▼
                  ┌──────────────────┐   ┌──────────────────┐
                  │ pm-strategy      │   │ pm-strategy      │
                  │ taker (module 2) │   │ maker (module 3) │
                  └────────┬─────────┘   └────────┬─────────┘
                           └──────────┬───────────┘
                                      ▼
                    ┌──────────────────────────────────────────────┐
                    │ pm-execution (module 4)                      │
                    │ DryRun (défaut) / SDK Rust officiel (live)   │
                    └──────────────────────────────────────────────┘

  Archives NDJSON (v2 + legacy) ──▶ pm-replay ──▶ mêmes types pm-core
                                    (strike-validate, cadence, backtest)
```

## Décisions de conception et justifications

### Rust + Tokio
- Latence : pas de GC, réveils précis, `tokio::select!` sur les WS.
- Le SDK CLOB **officiel** existe en Rust (`polymarket_client_sdk_v2`,
  repo `Polymarket/rs-clob-client-v2`) : signature EIP-712 locale + POST
  direct sur `https://clob.polymarket.com`, zéro couche intermédiaire.
- Continuité : la meilleure version legacy (collecte) était déjà en Rust ;
  la version python souffrait d'un cycle de collecte de 5–7 s.

### Acquisition (module 1) — « pas le droit à l'erreur »
- **Archive verbatim avant parsing** (`Recorder`, NDJSON v2) : les trames non
  reconnues ne sont jamais perdues (défaut du legacy corrigé) ; le format v2
  garde 3 horloges (murale, monotone, source) pour l'audit de latence.
- **Connexion RTDS continue**, indépendante des fenêtres : les ticks encadrant
  T0 sont toujours capturés. Le découpage par fenêtre est un post-traitement.
- **Réabonnement forcé après 5,5 s de silence** : comportement éprouvé par les
  deux versions legacy sur ce flux (cadence Chainlink parfois creuse).
- **CLOB par fenêtre** avec `custom_feature_enabled: true` pour recevoir
  `best_bid_ask` et surtout `market_resolved` (vérité terrain Up/Down qui
  auto-valide le strike reconstruit, fenêtre après fenêtre).
- **Watchdog** : tout flux silencieux > 6 s ⇒ `FeedStale` sur le bus ⇒ les
  stratégies coupent le risque (garde-fou câblé dans leurs `decide`).

### Price to beat (le point critique)
- Politique par défaut `LastAtOrBefore` : dernier update Chainlink avec
  `payload.timestamp ≤ T0` (justification : docs/PHASE1_FINDINGS.md §3).
- `StrikeComputation` porte **preuve** (ticks avant/après, gap) et
  **confidence** ∈ [0,1] ; sous le seuil configuré, aucun trade.
- Le strike est **gelé** dès qu'un tick ≥ T0 est observé (il ne peut plus
  changer par construction) — pas de dérive en cours de fenêtre.
- `pm-replay strike-validate` compare les 3 politiques aux valeurs affichées
  connues et aux `market_resolved` pour trancher définitivement sur données
  réelles.

### Volatilité (exclusivement flux de résolution)
- Rendements log normalisés par Δt (cadence Chainlink irrégulière) →
  variance **par seconde** ; agrégation : fenêtres réalisées 30 s–30 min et
  EWMA à temps irrégulier (λ^Δt, demi-vie 60 s).
- Projection σ·√τ pour l'horizon restant de la fenêtre.

### Stratégies (modules 2–3) — décisions PURES
- `MarketSnapshot` est la seule entrée : le live et le backtest exécutent
  exactement le même code (pm-replay produit les mêmes types).
- Modèle : diffusion log-normale avec drift optionnel (activé seulement si
  son signal domine le bruit), z-score = distance au strike en unités σ√τ.
- **Taker** : exige incohérence franche (|z| ≥ seuil ET edge ≥ seuil après
  coussin de coûts), profondeur vérifiée en marchant le carnet, taille = ¼
  Kelly plafonné. Fenêtres temporelles : jamais avant 10 s d'ancienneté de
  fenêtre, jamais sous 3 s restantes.
- **Maker** : quote uniquement dans [0.15, 0.85], bid ≤ fair − marge, TP à
  +0.08, stop à −0.10 sur le fair, liquidation forcée sous 25 s restantes,
  retrait si |z| > 2.5 (relais au taker) ou σ panique. Inventaire borné.
- ⚠️ Seuils par défaut = points de départ **à calibrer** sur les archives
  réelles ; les garde-fous d'intégrité, eux, ne se calibrent pas.

### Exécution (module 4)
- Trait `OrderGateway` ; `DryRunGateway` par défaut (paper + audit).
- Feature `live` : SDK officiel — le builder auto-résout tick size/neg-risk/
  fees (élimine des allers-retours et une classe d'erreurs). Types d'ordres :
  FAK/FOK pour le taker, GTC pour les quotes maker.
- Le passage en réel est un choix explicite de build + variables d'env,
  jamais un défaut.

## Validation incrémentale (état actuel)

| Étape | Moyen | Statut |
| --- | --- | --- |
| Parsing trames | fixtures verbatim des docs officielles | ✅ 70 tests workspace |
| Carnet L2 | tests unitaires snapshot/delta/depth | ✅ |
| Strike | 3 runs réels + relevés UI manuels | ✅ **5/5 exact, 9/9 gelés à T0** (docs/VALIDATION_LIVE.md) |
| Issue de résolution | market_resolved capturés vs issue estimée | ✅ **6/6 concordants** |
| Volatilité | processus synthétiques à σ connu + cadence réelle mesurée | ✅ |
| Stratégies | paper live avec PnL par fenêtre (PaperBroker) | ✅ taker net positif / ⏳ calibration maker |
| Acquisition live | 3 runs (75 min), pannes auto-récupérées | ✅ |
| Exécution live | feature `live` + petites tailles | ⏳ après campagne paper positive |
