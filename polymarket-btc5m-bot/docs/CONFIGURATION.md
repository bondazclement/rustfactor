# Référence de configuration — tous les paramètres

**Tout** le comportement du bot est configurable sans toucher au code, via
`config.toml` (créez-le avec `pm-ctl config init`, éditez avec
`pm-ctl config editer`). Ordre de priorité :

1. défauts calibrés du code (backtests du 04-05/07/2026) ;
2. `config.toml` (ou fichier passé via `--config`) — clés partielles OK ;
3. surcharges en ligne de commande (`--bankroll`, `--max-entry`, …).

Sécurités intégrées : une clé inconnue (faute de frappe) fait **échouer le
démarrage** avec un message explicite ; la configuration **effective** est
journalisée au démarrage ET archivée dans le journal NDJSON (`stream:
"meta"`) — chaque run est auditable après coup. Le même fichier alimente
`pm-bot` (live) et `pm-backtest` : testez vos réglages sur archives avant
de les mettre en live (`pm-ctl backtest`).

---

## `[taker]` — module d'entrée agressive (module 2)

| Clé | Défaut | Unité | Rôle | Surcharge CLI |
| --- | --- | --- | --- | --- |
| `bankroll` | 1000.0 | USDC | Capital de référence du calcul Kelly. Ne « bloque » rien : sert uniquement au dimensionnement. | `--bankroll` |
| `max_notional` | 250.0 | USDC | **Mise maximale par entrée** (plafond dur — c'est le montant réellement risqué par trade). | `--max-notional` |
| `kelly_fraction` | 0.25 | — | Fraction de Kelly appliquée (0.25 = quart de Kelly). | `--kelly` |
| `max_entry_price` | 0.85 | prob. | Prix d'achat maximal d'une part. Au-delà, l'asymétrie gain/perte devient dangereuse. | `--max-entry` |
| `min_abs_z` | 2.5 | σ | Incohérence minimale pour entrer. Automatiquement ×2 en tout début de fenêtre, ×1 sous 60 s. | `--min-z` |
| `min_edge` | 0.06 | prob. | Avantage net minimal : p(modèle) − prix payé − coûts. | `--min-edge` (backtest) |
| `cost_buffer` | 0.01 | prob. | Coussin forfaitaire frais + slippage retranché de l'edge. | — |
| `min_strike_confidence` | 0.8 | [0-1] | **Garde-fou** : pas de trade si le strike reconstruit est douteux. | — |
| `max_spot_age_ms` | 3000 | ms | **Garde-fou** : pas de trade si le dernier tick de résolution est trop vieux. | — |
| `min_elapsed_s` | 10.0 | s | Pas d'entrée dans les premières secondes d'une fenêtre. | — |
| `min_tau_s` | 3.0 | s | Pas d'entrée juste avant la résolution (latence d'exécution). | — |
| `max_slippage` | 0.02 | prob. | Écart max toléré entre meilleur ask et prix moyen d'exécution réel (profondeur). | — |

## `[maker]` — module de quotes passives (module 3, DÉSACTIVÉ par défaut)

⚠️ Perdant sur toute la grille au backtest (sélection adverse) — ne
s'active qu'explicitement (`pm-bot --maker` ou `pm-ctl direct --maker`).

| Clé | Défaut | Unité | Rôle |
| --- | --- | --- | --- |
| `quote_size` | 50.0 | parts | Taille d'une quote. |
| `max_inventory` | 150.0 | parts | Inventaire maximal par token. |
| `edge_margin` | 0.03 | prob. | Le bid reste au moins à cette marge sous le fair du modèle. |
| `take_profit` | 0.08 | prob. | Ask de sortie posé à entrée + TP. |
| `stop_loss` | 0.10 | prob. | Sortie forcée si le fair passe sous entrée − stop. |
| `min_quote_price` / `max_quote_price` | 0.15 / 0.85 | prob. | Zone quotable (pas d'extrêmes). |
| `min_tau_open_s` | 60.0 | s | Plus d'ouverture sous ce temps restant. |
| `min_tau_flat_s` | 25.0 | s | Liquidation totale sous ce temps restant. |
| `max_abs_z_quote` | 2.5 | σ | Au-delà, le maker se retire (relais au taker). |
| `sigma_panic_per_sqrt_s` | 5e-4 | σ/√s | Vol au-delà de laquelle les quotes sont retirées. |
| `min_strike_confidence` | 0.8 | [0-1] | **Garde-fou** (identique taker). |
| `max_spot_age_ms` | 3000 | ms | **Garde-fou** (identique taker). |

## `[modele]` — probabilité P(Up)

| Clé | Défaut | Rôle | Surcharge CLI (backtest) |
| --- | --- | --- | --- |
| `sigma_floor_per_sqrt_s` | 5e-5 | Plancher de volatilité (anti-z-infini par marché anormalement calme). | — |
| `tau_floor_s` | 1.0 | Temps restant plancher du calcul. | — |
| `drift_snr_min` | 0.25 | Le drift n'entre en jeu que s'il domine le bruit (fraction de σ√τ). | `--no-drift` (le coupe) |
| `max_drift_z` | 2.0 | **Plafond de contribution du drift au z** — empêche un choc récent extrapolé de fabriquer une fausse certitude (leçon du 04/07, −258 $). | `--drift-cap` |

## `[volatilite]` — reconstruction de σ (flux Chainlink uniquement)

| Clé | Défaut | Rôle |
| --- | --- | --- |
| `ewma_half_life_s` | 60.0 | Demi-vie de l'EWMA de variance (réactivité vs stabilité). |
| `retention_s` | 1800 | Historique de rendements conservé pour les fenêtres réalisées. |
| `min_dt_ms` | 10 | Intervalle minimal entre ticks (protection doublons d'horodatage). |

## `[moteur]` — orchestration

| Clé | Défaut | Rôle |
| --- | --- | --- |
| `decision_step_ms` | 250 | Cadence d'évaluation des stratégies. |
| `watchdog_stale_ms` | 6000 | Silence d'un flux avant état STALE (= zéro prise de risque). |
| `clob_grace_s` | 180 | Survie de la connexion CLOB après fin de fenêtre (capture de `market_resolved`). |
| `gamma_lookahead` | 5 | Fenêtres futures sondées à la découverte de marché. |
| `retention_ticks_s` | 2400 | Ticks de résolution gardés en mémoire (strike + vol longue). |

## Recettes

```bash
# Créer et personnaliser sa configuration
pm-ctl config init && pm-ctl config editer

# Dimensionner le capital sans toucher au fichier (surcharge ponctuelle)
pm-ctl direct --bankroll 5000 --max-notional 100

# Valider un réglage sur les archives AVANT de le mettre en live
pm-ctl backtest --config config.toml
pm-ctl backtest --max-entry 0.85          # comparaison ponctuelle

# La boucle de campagnes lit automatiquement config.toml s'il existe
pm-ctl demarrer --cible 200 --tranches 500
```

Note : les paramètres marqués **Garde-fou** protègent l'intégrité
(données périmées, strike douteux). Les assouplir n'augmente pas le
rendement — cela autorise seulement le bot à décider en aveugle. Ils sont
configurables comme le reste, mais y toucher devrait rester exceptionnel.
