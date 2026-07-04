# polymarket-btc5m-bot

Bot de trading pour l'événement Polymarket **« BTC 5mn Up/Down »**
(`btc-updown-5m-<epoch>`), refonte complète des versions historiques du repo.
Ce dossier est autonome et a vocation à devenir un repository indépendant.

## Règles absolues du projet

1. **Fidélité totale au flux de résolution natif Polymarket** : le price to
   beat exact et les fluctuations servant à reconstruire la volatilité
   proviennent exclusivement de RTDS `crypto_prices_chainlink` (`btc/usd`).
2. **Aucun flux externe** (Coinbase, Binance…) pour la résolution ou la
   volatilité — cause avérée de faux trades dans les versions précédentes.
3. **Archive avant parsing** : chaque trame réseau est journalisée verbatim
   avant toute interprétation (NDJSON v2).
4. **Garde-fous d'intégrité non négociables** : flux silencieux, strike
   douteux ou spot périmé ⇒ aucune prise de risque.

## Architecture (workspace Cargo)

| Crate | Rôle | Statut |
| --- | --- | --- |
| `pm-core` | Types, fenêtres, carnet L2, strike, volatilité, parsing pur | ✅ testé |
| `pm-acquisition` | Module 1 : RTDS + CLOB WS + Gamma + journal NDJSON v2 + watchdog | ✅ testé (fixtures docs) |
| `pm-replay` | Lecture archives (legacy + v2), CLI `strike-validate` / `cadence`, base du backtest | ✅ testé |
| `pm-strategy` | Module 2 (taker) + module 3 (market maker), décisions pures | ✅ testé, **à calibrer sur données réelles** |
| `pm-execution` | Module 4 : passerelle d'ordres — DryRun par défaut, SDK Rust officiel derrière la feature `live` | ✅ dry-run testé |
| `pm-bot` | Binaire d'orchestration (paper trading par défaut) | ✅ compile, à valider en réel |

Voir `docs/ARCHITECTURE.md` (conception détaillée et justification des choix)
et `docs/PHASE1_FINDINGS.md` (analyse des données legacy et du price to beat).

## Démarrage

```bash
cargo test --workspace          # 61 tests, aucun réseau requis
cargo run -p pm-bot             # paper trading (nécessite accès *.polymarket.com)
cargo run -p pm-bot -- --out ./data_v2 --no-maker

# Validation du strike sur une archive legacy :
cargo run -p pm-replay -- strike-validate \
  --legacy 'data_low_latency/window_1778341500/raw.ndjson' \
  --expected 80466.61
cargo run -p pm-replay -- cadence --legacy '.../raw.ndjson'
```

## Choix du langage

Rust + Tokio : latence minimale et prévisible (pas de GC), WebSockets natifs,
SDK CLOB officiel Polymarket en Rust (`polymarket_client_sdk_v2`) pour signer
et poster les ordres sans étape intermédiaire, et continuité avec le
collecteur legacy le plus performant (`Rustector_btc_5mn_1`).

## Ce qui manque encore (bloquants externes)

- **Les archives réelles** (`data_low_latency`, `data_5m`) ne sont pas dans le
  repo GitHub — les pousser (ou un échantillon) pour terminer la Phase 1 et
  calibrer les stratégies. Voir `docs/PHASE1_FINDINGS.md` §5.
- **Accès réseau** à `*.polymarket.com` depuis l'environnement d'exécution
  pour les tests live (actuellement bloqué par la politique réseau).
