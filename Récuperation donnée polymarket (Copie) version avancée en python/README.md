# Polymarket BTC Up/Down 5m Collector

Collecteur Python pour le marché **Bitcoin Up/Down 5m** de Polymarket : snapshots périodiques (par défaut 500 ms) dans un CSV par fenêtre de 5 minutes.

## Fonctionnalités

- Découverte du marché par **créneau UTC** : slug `btc-updown-5m-<epoch>` ; rotation avec intervalle court (défaut 5 s) et **sommeil ~0,8 s** près des frontières 5 minutes.
- **Strike sans Candlestick** :
  - `--strike` / `INITIAL_STRIKE_USD` pour la **première** fenêtre,
  - moteur RTDS (`crypto_prices_chainlink`) autour de `eventStartTime` avec score de confiance,
  - mode non bloquant : si non exact ou confiance insuffisante -> statut `pending_manual` + message **\"Strike à ajouter a posteriori\"**,
  - correction manuelle live via `strike-set` (persistée localement).
- **Spot BTC affiché** (colonne `btc_display`) : priorité au RTDS `btc/usd` Chainlink quand la donnée a moins de 5s, sinon bascule automatique vers RTDS `btcusdt` pour garder une vue live.
- **Interface interactive runtime** via **terminal séparé** : staging des modifications, validation sélective (`apply` partiel), application atomique sans arrêter le bot.
- WebSocket **CLOB** + repli HTTP last-trade pour UP/DOWN (takers).
- Volatilité 1m calculée sur candles BTC-USD Coinbase (close-to-close), avec fallback multi-source public.
- CSV et dashboard documentent la source effectivement affichée.

## Installation

```bash
python install.py
```

L'installateur crée la venv, installe les dépendances et lance le wizard de configuration.

## Lancement

```bash
.venv/bin/python -m collector.cli run
.venv/bin/python -m collector.cli run "https://polymarket.com/fr/event/btc-updown-5m-1774724400"
.venv/bin/python -m collector.cli run --strike 66848.13 "https://polymarket.com/..."
.venv/bin/python -m collector.cli strike-set 1774731900 66848.13
```

Commandes runtime (dans un **second terminal**):

```bash
.venv/bin/python -m collector.cli control
```

Exemples de commandes envoyables via `--command`:

```text
params
stage sample_interval_ms 200
stage ui_refresh_ms 120
staged
apply sample_interval_ms
apply
discard
set-strike 66848.13
```

## Format CSV

`timestamp_ms`, `readable_ts`, `window_epoch`, `window_slug`, `strike_price`, `strike_status`, `strike_source`, `strike_confidence`, `strike_age_ms`, `strike_before_ts_ms`, `strike_before_price`, `strike_after_ts_ms`, `strike_after_price`, `strike_gap_ms`, `strike_note`, `up_last_price`, `down_last_price`, `btc_display`, `btc_display_source`, `btc_display_age_ms`, `btc_spot_chainlink`, `btc_fast_usdt`, `btc_inst_vol_coinbase_1m`, `btc_inst_vol_source`, `btc_inst_vol_candle_count`, `btc_inst_vol_age_s`, `btc_inst_vol_status`.

## Diagnostics rapides

- **Gamma** : `curl "https://gamma-api.polymarket.com/events?slug=btc-updown-5m-$(($(date +%s)/300*300))"`
- **Override manuel** : `./.venv/bin/python -m collector.cli strike-set <epoch|slug|url> <strike>`
- **Fraîcheur spot** : la source affichée doit passer à `rtds_usdt_fallback` si `btc/usd` chainlink dépasse 5s d’âge.

## Securite credentials

- Les secrets restent en local dans `.env` (jamais dans Git).
- `.venv/`, `.env` et `node_modules/` sont ignores.
- `.env.example` ne contient que des placeholders sans secret.

Détails : [`LANCER_LE_BOT.md`](LANCER_LE_BOT.md), [`COINBASE_VOLATILITE_GUIDE.md`](COINBASE_VOLATILITE_GUIDE.md), [`.env.example`](.env.example).

## Rust low-latency collector (event-driven)

Un collecteur Rust orienté entraînement bot est disponible dans `Viewer rust rtds/`:

```bash
cd "Viewer rust rtds"
cargo run --bin low_latency_collector -- collect --out ../data_low_latency --duration-sec 320
```

Validation rapide des données d'une fenêtre:

```bash
cd "Viewer rust rtds"
cargo run --bin low_latency_collector -- validate --out ../data_low_latency --window-epoch <EPOCH>
```

Guide d’installation complet (prérequis, `install.sh`, compilation release, validation) : [`Viewer rust rtds/GUIDE_INSTALLATION_COLLECTEUR.md`](Viewer%20rust%20rtds/GUIDE_INSTALLATION_COLLECTEUR.md).

Voir aussi `Viewer rust rtds/LOW_LATENCY_COLLECTOR.md` pour le format des sorties (`raw.ndjson`, `features.ndjson`) et la méthode de benchmark.
