# Phase 1 — Exploration des données existantes : constats

> Statut : **conclusions VALIDÉES en conditions réelles le 2026-07-04** (voir
> docs/VALIDATION_LIVE.md — 5/5 strikes exacts vs UI). Les jeux de données legacy référencés
> restent utiles pour la non-régression mais ne sont plus bloquants.
> Constat d'origine : ils ne sont pas
> présents dans le repo GitHub (`bondazclement/rustfactor`, commit unique `229250a`).
> Ni `data_low_latency (copie de log de "Rustector_btc_5mn_1")/` ni
> `Récuperation donnée polymarket (Copie) version avancée en python/data_5m/` n'ont été poussés
> (très probablement à cause de la taille des fichiers ou d'un `.gitignore` local).
> L'analyse ci-dessous a donc été reconstruite **à partir du code source qui a produit ces fichiers**
> (les writers sont déterministes : le format est connu à 100 %), en attendant les données réelles.

## 1. Format de `window_<epoch>/raw.ndjson` (Rustector_btc_5mn_1)

Producteur : `Rustector_btc_5mn_1/src/record.rs` (`RecordEvent`, sérialisé serde avec
`tag = "kind"`, `rename_all = "snake_case"`, une ligne JSON par événement).

Trois types de lignes :

```jsonc
// 1. Changement de fenêtre (ouverture du fichier)
{"kind":"window_changed","ts_ms":1778341500123,"window_slug":"btc-updown-5m-1778341500",
 "window_epoch":1778341500,"token_up":"<clob_token_id>","token_down":"<clob_token_id>"}

// 2. Tick RTDS Chainlink (flux de résolution)
{"kind":"rtds_tick","ts_ms":1778341501456,      // réception locale (horloge machine)
 "source_ts_ms":1778341501380,                  // payload.timestamp = horodatage Chainlink
 "message_ts_ms":1778341501440,                 // timestamp du message RTDS
 "symbol":"btc/usd","price":80466.61,"raw_value":80466.61}

// 3. Événement CLOB (payload complet recopié)
{"kind":"clob_event","ts_ms":...,"event_type":"book|price_change|last_trade_price|best_bid_ask",
 "event_ts_ms":...,"payload":{ /* message CLOB verbatim */ }}
```

Points de fidélité du format legacy :
- Le payload CLOB est conservé **verbatim** (bien) mais seulement s'il a été parsé avec succès
  (les messages non reconnus sont **silencieusement perdus** — défaut corrigé dans la refonte).
- Les ticks RTDS `crypto_prices` (btcusdt/Binance) ne sont **pas** enregistrés, seulement affichés.
- Trois horloges par tick : réception locale / timestamp source Chainlink / timestamp message RTDS.
  C'est exactement ce qu'il faut pour raisonner sur la latence et sur la frontière de fenêtre.

## 2. Format de `data_5m/btc_<epoch>.csv` (version python)

Producteur : `collector/csv_writer.py` (`CSV_HEADER`, 37 colonnes) — une ligne par cycle de
collecte (~5-7 s dans cette version, ce qui était le problème principal).
Colonnes clés : `strike_price`, `strike_status` (`exact_auto|approx_auto|pending_manual`),
`strike_source` (`rtds_exact_tick|rtds_interpolated_boundary|rtds_first_after|rtds_last_before`),
`strike_confidence`, `strike_before/after_ts_ms`, `strike_before/after_price`, `strike_gap_ms`,
plus spot Chainlink, spot btcusdt, et colonnes de volatilité Coinbase (sans valeur pour la
résolution — confirmé par le contexte).

## 3. Le « price to beat » : diagnostic des deux écarts observés

### Mécanique côté Polymarket (docs officielles, MCP)
- Les marchés crypto courts (`btc-updown-5m-<epoch>`) se résolvent sur le flux **Chainlink**
  (Data Streams) que Polymarket rediffuse via RTDS `wss://ws-live-data.polymarket.com`,
  topic `crypto_prices_chainlink`, symbole `btc/usd`.
- **Aucune API publique n'expose le strike** : ni `gamma-openapi.yaml` ni `clob-openapi.yaml`
  ne contiennent de champ `openPrice`/`strike`/`priceToBeat`. Le chiffre affiché par l'UI est
  dérivé du même flux. Il faut donc le **reconstruire**, et c'est le point « pas droit à l'erreur ».

### Fenêtre 1778341500 (Rustector) — « price to beat ≠ open price »
Le « price to beat » affiché ($80,466.61) diffère du premier tick *dans* la fenêtre.
Interprétation : le strike de Polymarket est la **dernière valeur Chainlink connue à T0**
(dernier update avec `payload.timestamp ≤ début de fenêtre`), pas le premier update reçu après
l'ouverture. Entre deux updates Chainlink, le prix « courant » à T0 est celui du dernier update —
c'est ce que l'UI fige comme price to beat.

### Fenêtre 1778343900 (python) — enregistré $80,714.93 vs réel $80,714.87
`strike_resolver.py` **interpole linéairement** entre le dernier tick avant T0 et le premier
tick après T0 (`rtds_interpolated_boundary`). L'écart de +6 cents est exactement la signature
d'une interpolation vers un tick suivant plus haut : la vraie valeur ($80,714.87) correspond
au **tick avant la frontière**, pas au point interpolé.
(Cohérent aussi avec le constat « la version python était en moyenne + exacte » : elle
échantillonnait autour de T0 et retenait un point proche du dernier tick ≤ T0, là où d'autres
versions prenaient le premier tick ≥ T0.)

### Politique retenue pour la refonte — ✅ VALIDÉE EN RÉEL le 2026-07-04
(5/5 fenêtres exactes vs UI, 0,00 $ d'écart — voir docs/VALIDATION_LIVE.md)
```
strike(T0) = prix du dernier update crypto_prices_chainlink btc/usd
             avec payload.timestamp ≤ T0
```
- Implémentée dans `pm-core::strike` comme politique par défaut (`LastAtOrBefore`), avec les
  politiques alternatives (`FirstAfter`, `Interpolate`) conservées pour le banc de validation.
- `pm-replay strike-validate` rejoue chaque `raw.ndjson` et compare les politiques entre elles
  et aux valeurs affichées connues (ex. 1778341500 → 80466.61 ; 1778343900 → 80714.87), ainsi
  qu'aux événements `market_resolved` du canal CLOB (issue Up/Down = vérité terrain du signe).
- Le collecteur v2 s'abonne en continu (pas de rotation à T0) pour ne **jamais** manquer les
  ticks encadrant la frontière, et journalise chaque trame brute verbatim avant tout parsing.

## 4. Défauts du legacy corrigés par la refonte

| Défaut observé | Correction v2 |
| --- | --- |
| Trames non parsées = perdues (Rust) | Journal **raw verbatim** (`{recv_ms, stream, raw}`) écrit avant parsing |
| Cadence 5-7 s (python) | Push WS natif, zéro polling sur le chemin chaud |
| Volatilité via Coinbase (python) | Volatilité reconstruite exclusivement depuis les ticks Chainlink RTDS |
| Strike interpolé (python) | Politique « dernier tick ≤ T0 » + evidence + confidence + banc de validation |
| Ticks btcusdt non archivés | Archivés (utile comme indicateur avancé, jamais pour la résolution) |
| Rotation de fichier au changement de fenêtre via Gamma (peut rater la frontière) | Enregistrement continu + découpage par fenêtre au post-traitement ; frontières calculées sur `payload.timestamp`, pas sur l'heure de rotation |

## 5. Données legacy (plus bloquantes — utiles pour la non-régression)

L'hypothèse ayant été validée en réel, ces fichiers servent désormais uniquement à étendre
l'historique de validation :
1. `data_low_latency (copie de log de "Rustector_btc_5mn_1")/window_1778341500/raw.ndjson`
   (au minimum les 1 000 premières lignes) — validation du strike $80,466.61 et de la cadence
   réelle des ticks Chainlink (inter-arrivées, jitter, doublons éventuels).
2. `Récuperation donnée polymarket (Copie) version avancée en python/data_5m/btc_1778343900.csv`
   — validation de l'hypothèse « tick avant frontière = 80,714.87 » via les colonnes
   `strike_before_price` / `strike_after_price`.

> Si les fichiers dépassent les limites GitHub : `git lfs`, ou un échantillon
> (`head -n 5000 raw.ndjson`) suffit pour la validation.

(Historique : la politique réseau bloquait initialement `*.polymarket.com` ; elle a été ouverte
le 2026-07-04 et un tunnel proxy CONNECT a été intégré au client WebSocket — les tests live
passent désormais depuis l'environnement.)
