# Lancer le bot de A a Z

Guide complet pour demarrer le bot Polymarket BTC 5m, avec les commandes exactes.

## 1) Aller dans le projet

```bash
cd "/home/clementbondaz-sanson/Documents/Projet code/Récuperation donnée polymarket"
```

## 2) Creer et preparer l'environnement Python

Option recommandee (si `.venv` n'existe pas encore) :

```bash
python3 -m venv .venv
./.venv/bin/pip install --upgrade pip
./.venv/bin/pip install -e .
```

Option alternative :

```bash
python3 install.py
```

## 3) Initialiser le fichier de configuration

Si tu n'as pas encore de `.env` :

```bash
cp .env.example .env
```

## 4) Configurer `.env` (minimum utile)

Tu peux ouvrir le fichier :

```bash
nano .env
```

Variables importantes :

- `INITIAL_STRIKE_USD` : strike manuel pour la premiere fenetre uniquement.
- `STRIKE_OVERRIDES_PATH` : fichier des corrections manuelles a posteriori.
- `STRIKE_CONFIDENCE_THRESHOLD` : seuil de confiance pour accepter `approx_auto` (si activé).
- `STRIKE_CONFIDENCE_GAP_MS` : gap max de reference pour le score de confiance RTDS.
- `STRIKE_ALLOW_APPROX_AUTO` : `false` recommande (sinon le bot peut accepter auto un strike non exact).
- `STRIKE_EXACT_TOLERANCE_MS` : tolerance timestamp pour marquer `exact_auto`.
- `DISCOVERY_INTERVAL_SECONDS` : poll des nouvelles fenetres (defaut 5).
- `SAMPLE_INTERVAL_MS` : intervalle d'ecriture CSV.
- `UI_REFRESH_MS` : frequence de refresh de l'interface.
- `RTDS_FAST_BTCUSDT` : garder `true` pour fallback auto quand la source chainlink RTDS est stale (>5s).
- `COINBASE_VOL_WINDOW_CANDLES` : nombre de bougies 1m utilisees pour la volatilite.
- `COINBASE_POLL_INTERVAL_SECONDS` : frequence de polling des candles Coinbase.
- `COINBASE_SOURCE_MODE` : `auto` (recommande), `exchange`, ou `advanced`.
- `RUNTIME_CONTROL_SOCKET_PATH` : socket locale pour le menu control separe.

## 5) Lancer le bot (mode normal)

```bash
./.venv/bin/python -m collector.cli run
```

## 6) Lancer avec strike manuel pour la premiere fenetre

```bash
./.venv/bin/python -m collector.cli run --strike 66848.13
```

## 7) Lancer sur un slug ou une URL specifique

```bash
./.venv/bin/python -m collector.cli run "https://polymarket.com/fr/event/btc-updown-5m-1774724400"
```

ou

```bash
./.venv/bin/python -m collector.cli run --slug "https://polymarket.com/fr/event/btc-updown-5m-1774724400"
```

## 8) Verifier que le bot tourne correctement

Dans l'UI, verifier :

- `Strike:` affiche un prix.
- Si statut `pending_manual`, le bot affiche le message **`Strike à ajouter a posteriori`**.
- Ligne `Displayed BTC spot` visible avec `source=...`.
- Ligne `BTC/USD (Polymarket RTDS Chainlink)` visible.
- Si age chainlink depasse 5s, la source affichee doit passer a `rtds_usdt_fallback`.
- Les ages (`age=`) evoluent et restent faibles.

## 8 bis) Corriger un strike a posteriori (sans stopper le bot)

```bash
./.venv/bin/python -m collector.cli strike-set 1774731900 66848.13
```

Tu peux aussi passer un slug ou une URL:

```bash
./.venv/bin/python -m collector.cli strike-set "btc-updown-5m-1774731900" 66848.13
./.venv/bin/python -m collector.cli strike-set "https://polymarket.com/fr/event/btc-updown-5m-1774731900" 66848.13
```

## 8 ter) Modifier des parametres a chaud (terminal separe)

Pendant que le bot tourne, ouvre un second terminal et lance:

```bash
./.venv/bin/python -m collector.cli control
```

Ou en mode commande unique:

```bash
./.venv/bin/python -m collector.cli control --command "params"
./.venv/bin/python -m collector.cli control --command "stage sample_interval_ms 200"
./.venv/bin/python -m collector.cli control --command "apply sample_interval_ms"
```

Commandes disponibles:

```text
help
params
stage sample_interval_ms 200
stage ui_refresh_ms 120
staged
apply sample_interval_ms
apply
discard
set-strike 66848.13
```

Points importants:

- Les changements `stage` ne sont pas appliques tant que tu ne fais pas `apply`.
- `apply` sans argument applique tout le staging.
- `apply <param1> <param2>` applique uniquement ce sous-ensemble.
- `discard` annule tout le staging en attente.

## 9) Verifier les fichiers de sortie

Les CSV sont dans `data_5m/` :

```bash
ls data_5m
```

Les logs sont dans `logs/` :

```bash
ls logs
```

## 10) Commandes de diagnostic utiles

Verifier le slug courant (fenetre UTC 5 min) :

```bash
curl -sS "https://gamma-api.polymarket.com/events?slug=btc-updown-5m-$(($(date +%s)/300*300))"
```

Afficher l'aide CLI :

```bash
./.venv/bin/python -m collector.cli --help
```

Lancer un test court (15s) :

```bash
timeout 15s ./.venv/bin/python -m collector.cli run --strike 66848.13
```

## 11) Arreter le bot

- Dans le terminal du bot : `Ctrl+C`

## 12) Probleme courant et correction rapide

- **Source affichee souvent `rtds_usdt_fallback`** : le flux `crypto_prices_chainlink` est trop sparse; le fallback est normal.
- **Pas de mise a jour rapide** : verifier `RTDS_FAST_BTCUSDT=true`.
- **Strike en `pending_manual`** : injecter le strike exact via `strike-set` puis verifier que le statut passe en `exact_manual`.

## 13) Hygiene securite

- Ne jamais commit `.env` ni `.venv`.
- Laisser les cles API dans `.env` local uniquement.
- Partager uniquement `.env.example` (sans secrets).
