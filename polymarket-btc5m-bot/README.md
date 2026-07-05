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
| `pm-core` | Types, fenêtres, carnet L2, strike, volatilité, parsing pur | ✅ testé + **validé live** |
| `pm-acquisition` | Module 1 : RTDS + CLOB WS + Gamma + journal NDJSON v2 + watchdog + proxy CONNECT | ✅ **validé live** (3 runs, 75 min, auto-récupération pannes) |
| `pm-replay` | Lecture archives (legacy + v2), CLI `strike-validate` / `cadence`, base du backtest | ✅ testé |
| `pm-strategy` | Module 2 (taker) + module 3 (market maker) + PaperBroker | ✅ testé + paper live, **calibration maker en cours** |
| `pm-execution` | Module 4 : passerelle d'ordres — DryRun par défaut, SDK Rust officiel derrière la feature `live` | ✅ dry-run validé live ; live jamais activé |
| `pm-bot` | Binaire d'orchestration : paper trading + PnL par fenêtre + validation auto vs résolutions officielles | ✅ **validé live** |

**Résultats clés des runs réels du 2026-07-04** (détail : `docs/VALIDATION_LIVE.md`) :
price to beat reconstruit **exact 5/5** vs l'affichage Polymarket (écart 0,00 $),
issue estimée **6/6** concordante avec les résolutions officielles `market_resolved`,
strike gelé à T0 avec confidence 1.0 sur **9/9** fenêtres. Archives brutes de
référence dans `data_samples/` (zstd).

Voir `docs/ARCHITECTURE.md` (conception détaillée et justification des choix)
et `docs/PHASE1_FINDINGS.md` (analyse des données legacy et du price to beat).

## Démarrage rapide (Fedora / machine locale)

```bash
git clone https://github.com/bondazclement/rustfactor.git && cd rustfactor/polymarket-btc5m-bot && ./install.sh
pm-ctl sante && pm-ctl demarrer     # dry run supervisé : pm-ctl statut
```
Guide complet : `docs/INSTALLATION_FEDORA.md` · Pilotage : `pm-ctl aide` ·
Configuration (tout paramètre) : `docs/CONFIGURATION.md` + `config.exemple.toml` ·
Audit de robustesse : `docs/AUDIT_ROBUSTESSE.md`

## Démarrage (développement)

```bash
cargo test --workspace          # 72 tests, aucun réseau requis
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

## État et prochaines étapes

- ✅ Accès réseau `*.polymarket.com` ouvert dans l'environnement ; module net
  avec tunnel proxy CONNECT intégré au client WebSocket.
- ✅ Hypothèse strike **validée en réel** — les archives legacy ne sont plus
  bloquantes (elles restent bienvenues pour étendre l'historique).
- ⏳ Calibration du maker (porte de l'inventaire au règlement — voir
  `docs/VALIDATION_LIVE.md`), puis campagne longue, puis backtest replay.
- ⛔ Passage en réel (`--features live`) : seulement après calibration
  positive démontrée sur campagne paper longue.
