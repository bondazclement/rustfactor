# Guide volatilite Coinbase (lente + temps reel)

Le collecteur maintient deux signaux complementaires:

1. **Volatilite 1m close-to-close (lente / regime)** via REST candles Coinbase.
2. **Volatilite temps reel (ressenti trader)** via WebSocket `matches` Coinbase Exchange.

Les deux sont ecrites dans le CSV pour entrainement.

## Sources utilisees

- **REST (lente, robuste)**
  - Priorite 1: `https://api.exchange.coinbase.com/products/BTC-USD/candles?granularity=60`
  - Fallback: `https://api.coinbase.com/api/v3/brokerage/market/products/BTC-USD/candles?granularity=ONE_MINUTE`
- **WS (temps reel)**
  - `wss://ws-feed.exchange.coinbase.com` + subscription `matches` sur `BTC-USD`

Par defaut, aucune cle API n'est requise.

## Logique de calcul

### 1) Signal lent (historique 1m)

1. Recuperation des candles BTC-USD 1m.
2. Exclusion de la bougie en cours.
3. Tri + deduplication par timestamp.
4. Prise des `N` dernieres closes (`COINBASE_VOL_WINDOW_CANDLES`).
5. Log-returns close-to-close.
6. Ecart-type population.

Sortie principale: `btc_inst_vol_coinbase_1m`.

### 2) Signal temps reel (ressenti)

1. Ingestion des trades `match` BTC-USD.
2. Aggregation en barres courtes (`COINBASE_RT_BAR_MS`) pour limiter le bruit microstructure.
3. Calcul des features:
   - `btc_rt_vol_ewma`: vol EWMA des returns de barres.
   - `btc_rt_vol_sigma_30s`: sigma sur fenetre courte.
   - `btc_rt_vol_sigma_120s`: sigma sur fenetre plus large.
   - `btc_rt_candle_range_rel_1m`: `(high-low)/mid` de la minute UTC en cours.
   - `btc_rt_candle_log_ret_open_1m`: `log(last/open)` de la minute UTC en cours.
   - `btc_rt_trade_count_60s`: nombre de trades vus sur 60s.
4. Calcul d'etat/fraicheur:
   - `btc_rt_age_ms`: age du dernier trade.
   - `btc_rt_status`: `ok`, `insufficient_samples`, `stale`, `ws_disconnected`, `initializing`, `disabled`.

## Parametres .env

### REST (signal lent)

- `COINBASE_VOL_WINDOW_CANDLES` (defaut 60)
- `COINBASE_POLL_INTERVAL_SECONDS` (defaut 10)
- `COINBASE_SOURCE_MODE` (`auto`, `exchange`, `advanced`)
- `COINBASE_PRODUCT_ID` (defaut `BTC-USD`)

### WS (signal temps reel)

- `COINBASE_WS_URL` (defaut `wss://ws-feed.exchange.coinbase.com`)
- `COINBASE_RT_ENABLED` (`true`/`false`)
- `COINBASE_RT_BAR_MS` (defaut `200`)
- `COINBASE_RT_EWMA_HALFLIFE_S` (defaut `12`)
- `COINBASE_RT_WINDOW_FAST_S` (defaut `30`)
- `COINBASE_RT_WINDOW_SLOW_S` (defaut `120`)
- `COINBASE_RT_STALE_AFTER_MS` (defaut `6000`)

## Colonnes CSV

### Signal lent

- `btc_inst_vol_coinbase_1m`
- `btc_inst_vol_source`
- `btc_inst_vol_candle_count`
- `btc_inst_vol_age_s`
- `btc_inst_vol_status`

### Signal temps reel

- `btc_rt_vol_ewma`
- `btc_rt_vol_sigma_30s`
- `btc_rt_vol_sigma_120s`
- `btc_rt_candle_range_rel_1m`
- `btc_rt_candle_log_ret_open_1m`
- `btc_rt_trade_count_60s`
- `btc_rt_age_ms`
- `btc_rt_status`

## Verification manuelle rapide

1. Lancer le collecteur et attendre 1-2 minutes.
2. Verifier dans le CSV:
   - les colonnes `btc_rt_*` bougent en continu;
   - `btc_rt_status` est majoritairement `ok`;
   - `btc_rt_age_ms` reste bas (hors coupure reseau).
3. Verifier que les colonnes `btc_inst_vol_*` continuent d'etre remplies pour le contexte regime.
