# polymarket-viewer — Guide complet

Interface CLI temps réel pour le marché **btc-updown-5m** de Polymarket.
Affiche les prix Up/Down pour takers, le prix BTC Chainlink en live, l'orderbook complet, et la bougie courante.

---

## Prérequis système

- Linux x86_64 (testé : Fedora 43)
- Connexion internet
- Terminal d'au moins **120×35** caractères recommandé

Aucune dépendance C requise — le projet utilise **rustls** (TLS pur Rust).

---

## Installation — one command

```bash
bash install.sh
```

Ce script fait tout automatiquement :
1. Vérifie / installe Rust via `rustup`
2. Met à jour la toolchain stable
3. Installe `rustfmt` et `clippy`
4. Vérifie les certificats CA
5. Compile le binaire release optimisé
6. Crée un lien symbolique dans `~/.local/bin` si disponible dans PATH

---

## Commandes de A à Z

### 1. Cloner / accéder au projet

```bash
cd ~/Documents/Projet\ code/Viewer\ rust\ rtds
```

### 2. Installer (première fois)

```bash
bash install.sh
```

### 3. Lancer le viewer

**Via le binaire release (recommandé — le plus rapide) :**
```bash
./target/release/polymarket-viewer
```

**Via Cargo (recompile si modifié) :**
```bash
cargo run --release
```

**Via lien global (si `~/.local/bin` est dans PATH) :**
```bash
polymarket-viewer
```

### 4. Quitter

Appuyer sur `q`, `Q` ou `Échap` dans le terminal.

---

## Mise à jour du code

```bash
# Après modification des sources :
cargo build --release
./target/release/polymarket-viewer
```

---

## Commandes de développement

```bash
# Compilation debug (plus rapide, moins optimisé)
cargo build

# Lancer en mode debug
cargo run

# Vérifier le code sans compiler
cargo check

# Linter
cargo clippy

# Formater le code
cargo fmt

# Voir les warnings en détail
cargo build 2>&1 | grep warning
```

## Collecteur low-latency (dataset bot)

Un binaire dédié de collecte événementielle est disponible pour la capture brute:

```bash
# Collecte (exemple 5 minutes + marge)
cargo run --bin low_latency_collector -- collect --out ../data_low_latency --duration-sec 320

# Validation d'une fenêtre
cargo run --bin low_latency_collector -- validate --out ../data_low_latency --window-epoch <EPOCH>
```

Guide d’installation et toutes les commandes : `GUIDE_INSTALLATION_COLLECTEUR.md`.  
Architecture et format NDJSON : `LOW_LATENCY_COLLECTOR.md`.

---

## Structure du projet

```
Viewer rust rtds/
├── Cargo.toml              # Dépendances Rust
├── Cargo.lock              # Versions verrouillées
├── install.sh              # Installateur one-command
├── GUIDE.md                # Ce fichier
└── src/
    ├── bin/
    │   └── low_latency_collector.rs  # Collecteur événementiel (dataset bot)
    ├── main.rs             # Entrypoint, orchestration des tâches async
    ├── models.rs           # Structs de données (orderbook, candle, état global)
    ├── gamma.rs            # Découverte du marché actif via Gamma API
    ├── ws_clob.rs          # WebSocket CLOB (orderbook temps réel)
    ├── ws_rtds.rs          # WebSocket RTDS (Chainlink + Binance BTC)
    └── ui.rs               # Interface ratatui
```

---

## Ce que vous voyez à l'écran

```
┌──────────────────────────────────────────────────────────────────────────┐
│ ⚡ Bitcoin Up or Down - March 30, ... [04:12]  Chainlink: $67428  Binance: $67431 │
├─────────────────────────────┬─────────────────────────────┬──────────────┤
│  UP ▲ | Bid:0.490 Ask:0.500 │ DOWN ▼ | Bid:0.490 Ask:0.500│ Live Candle  │
│  Price   Size ($)  Side     │  Price   Size ($)  Side     │  Since: ...  │
│  0.510   250.0    SELL      │  0.510   180.0    SELL      │  Open: 0.500 │
│  0.500   150.0    SELL      │  0.500   120.0    SELL      │  High: 0.510 │
│ ──0.495──          mid      │ ──0.495──          mid      │  Low:  0.490 │
│  0.490   200.0    BUY       │  0.490   300.0    BUY       │  Close: 0.50 │
│  0.480   400.0    BUY       │  0.480   500.0    BUY       │  Vol: $45.2  │
│                             │                             │  ▲ UP        │
│                             │                             ├──────────────┤
│                             │                             │ Recent Trades│
│                             │                             │ 01:45 0.5000 │
│                             │                             │ 01:45 0.4900 │
├─────────────────────────────┴─────────────────────────────┴──────────────┤
│ CLOB: ✓ 123ms lag   RTDS: ✓ 956ms lag   Local: 01:45:32.418   [q] Quit  │
└──────────────────────────────────────────────────────────────────────────┘
```

**Colonnes orderbook :**
- `Price` — prix du token Up ou Down (entre 0 et 1, représente la probabilité)
- `Size ($)` — taille en dollars de la liquidité à ce niveau
- `Side` — SELL (asks) en rouge / BUY (bids) en vert

**Interpréter les prix pour un taker :**
- Pour acheter **UP** : regarder le meilleur **Ask** de l'orderbook UP
- Pour acheter **DOWN** : regarder le meilleur **Ask** de l'orderbook DOWN
- Prix UP + Prix DOWN ≈ 1.00 (frais inclus)

**Source de résolution :** Chainlink BTC/USD — le prix affiché en header est exactement celui qui détermine si le marché résout UP ou DOWN.

---

## Données transmises par Chainlink

Le prix BTC affiché vient du stream Chainlink `btc/usd` via le RTDS Polymarket.
C'est la même source que `https://data.chain.link/streams/btc-usd`.

Champs reçus :
| Champ | Description |
|-------|-------------|
| `value` | Prix BTC en USD (float64) |
| `full_accuracy_value` | Prix haute précision (string, 18 décimales) |
| `timestamp` | Moment de la mesure Chainlink (Unix ms) |
| `timestamp` (RTDS) | Moment de réception par RTDS (Unix ms) |

La latence affichée dans le footer = `heure réception` − `timestamp RTDS`.
La latence totale depuis Chainlink est généralement de **900ms à 1500ms**.

---

## Dépannage

**Le terminal s'affiche mal (artefacts visuels) :**
```bash
reset
cargo run --release
```

**Erreur "market not found" au démarrage :**
Normal si la fenêtre de 5 min vient de se terminer — le programme cherche les slots suivants automatiquement. Attendez quelques secondes.

**Connexion WebSocket perdue :**
Les deux WebSockets se reconnectent automatiquement avec 2 secondes de délai.

**Recompiler après mise à jour :**
```bash
cargo build --release
```

---

## Désinstaller

```bash
# Supprimer le lien symbolique global (si créé)
rm -f ~/.local/bin/polymarket-viewer

# Supprimer les artefacts de compilation (libère ~500MB)
cargo clean

# Supprimer le projet entier
rm -rf ~/Documents/Projet\ code/Viewer\ rust\ rtds
```
