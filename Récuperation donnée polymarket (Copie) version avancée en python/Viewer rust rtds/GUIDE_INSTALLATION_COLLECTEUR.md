# Guide d’installation et d’utilisation — collecteur low-latency (Rust)

Ce document regroupe **toutes les commandes** nécessaires pour installer, compiler et lancer le binaire **`low_latency_collector`**, ainsi que les **références au code source** (fichiers et constantes utiles). Il complète [`LOW_LATENCY_COLLECTOR.md`](LOW_LATENCY_COLLECTOR.md) (architecture et format des données).

---

## 1. Contenu du programme

| Composant | Rôle |
|-----------|------|
| **`low_latency_collector`** | Binaire Rust : collecte événementielle RTDS Chainlink `btc/usd` + WebSocket CLOB (Up/Down), écrit `raw.ndjson` et `features.ndjson` par fenêtre 5 min. |
| **`polymarket-viewer`** | Interface TUI (autre binaire du même crate) — voir [`GUIDE.md`](GUIDE.md). |

**Fichier source principal du collecteur** (toute la logique est dans ce fichier) :

- [`src/bin/low_latency_collector.rs`](src/bin/low_latency_collector.rs)

**Endpoints et symboles codés en dur** (extrait du début du fichier) :

```rust
const GAMMA_BASE: &str = "https://gamma-api.polymarket.com";
const RTDS_WS: &str = "wss://ws-live-data.polymarket.com";
const CLOB_WS: &str = "wss://ws-subscriptions-clob.polymarket.com/ws/market";
const CHAINLINK_SYMBOL: &str = "btc/usd";
```

**Dépendances Rust** : voir [`Cargo.toml`](Cargo.toml) (`tokio`, `tokio-tungstenite`, `reqwest`, `serde_json`, etc.).

---

## 2. Prérequis système

- **Linux** (testé sur Fedora ; fonctionne aussi sur Debian/Ubuntu avec les mêmes outils).
- **Connexion Internet** (Gamma API + WebSockets Polymarket).
- **Certificats racine TLS** : le projet utilise **rustls** ; installez `ca-certificates` si besoin (voir `install.sh`).
- **Rust / Cargo** : stable recommandé (via [rustup](https://rustup.rs/)).

Installation minimale de Rust si vous n’avez rien :

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
source "$HOME/.cargo/env"
rustc --version
cargo --version
```

---

## 3. Se placer dans le bon répertoire

Le projet se trouve sous un chemin avec **espaces** ; utilisez des guillemets ou l’échappement.

```bash
cd "/home/clementbondaz-sanson/Documents/Projet code/Récuperation donnée polymarket/Viewer rust rtds"
```

(Remplacez par votre chemin réel vers le dossier **`Viewer rust rtds`**.)

---

## 4. Installation « tout-en-un » (viewer + toolchain)

Le script **`install.sh`** installe ou met à jour Rust, vérifie les CA, et compile le crate en **release** (binaire **`polymarket-viewer`**).

```bash
cd "/chemin/vers/Viewer rust rtds"
bash install.sh
```

À la fin, vous obtenez notamment :

- `./target/release/polymarket-viewer`
- Optionnellement un lien `~/local/bin/polymarket-viewer` si `~/.local/bin` est dans votre `PATH`.

**Important** : `install.sh` ne compile pas explicitement le binaire **`low_latency_collector`**, mais le **même `cargo build --release`** compile tout le crate. Pour être sûr d’avoir le collecteur :

```bash
cd "/chemin/vers/Viewer rust rtds"
cargo build --release --bin low_latency_collector
```

---

## 5. Compilation sans `install.sh` (manuelle)

```bash
cd "/chemin/vers/Viewer rust rtds"

# Vérification rapide (debug, plus rapide à compiler)
cargo check --bin low_latency_collector

# Binaire debug
cargo build --bin low_latency_collector

# Binaire optimisé (recommandé pour la collecte longue)
cargo build --release --bin low_latency_collector
```

Emplacements des binaires :

- Debug : `target/debug/low_latency_collector`
- Release : `target/release/low_latency_collector`

---

## 6. Lancer le collecteur (`collect`)

### Syntaxe

```text
low_latency_collector collect [--out DIR] [--duration-sec N]
```

- **`--out DIR`** : répertoire racine des données (défaut : `data_low_latency` relatif au répertoire courant).
- **`--duration-sec N`** : arrêt automatique après **N** secondes. **Si omis**, le programme tourne jusqu’à **Ctrl+C**.

### Exemples (depuis `Viewer rust rtds/`)

Collecte **5 minutes + marge** (une fenêtre complète environ) :

```bash
cargo run --release --bin low_latency_collector -- collect --out ../data_low_latency --duration-sec 320
```

Collecte **sans limite de temps** (arrêt manuel) :

```bash
cargo run --release --bin low_latency_collector -- collect --out ../data_low_latency
```

Avec le binaire release déjà compilé :

```bash
./target/release/low_latency_collector collect --out ../data_low_latency --duration-sec 320
```

Au démarrage, le programme affiche le **slug** du marché actif (ex. `btc-updown-5m-<epoch>`). L’**epoch** est le nombre `<epoch>` dans ce slug — utile pour la validation (section 7).

---

## 7. Valider une fenêtre (`validate`)

Après collecte, les données sont sous :

```text
<out>/window_<epoch>/raw.ndjson
<out>/window_<epoch>/features.ndjson
```

### Syntaxe

```text
low_latency_collector validate [--out DIR] --window-epoch N
```

- **`--window-epoch N`** : **obligatoire** — l’epoch entier (ex. `1774925400`), pas le slug complet.

### Exemple

```bash
cargo run --release --bin low_latency_collector -- validate --out ../data_low_latency --window-epoch 1774925400
```

### Trouver l’epoch si vous ne l’avez pas noté

```bash
EPOCH=$(($(date +%s) / 300 * 300))
curl -sS -A "Mozilla/5.0" "https://gamma-api.polymarket.com/events?slug=btc-updown-5m-${EPOCH}" | head -c 200
```

Ou lister les dossiers créés :

```bash
ls -d ../data_low_latency/window_*
```

Le nom du dossier est `window_<epoch>`.

---

## 8. Récapitulatif des commandes (copier-coller)

Séquence type **première installation + collecte + validation** :

```bash
# 1) Rust (si besoin)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
source "$HOME/.cargo/env"

# 2) Aller dans le crate Rust
cd "/home/clementbondaz-sanson/Documents/Projet code/Récuperation donnée polymarket/Viewer rust rtds"

# 3) (Optionnel) installateur viewer + toolchain
bash install.sh

# 4) Compiler le collecteur en release
cargo build --release --bin low_latency_collector

# 5) Collecter ~5 min 20 s de données
./target/release/low_latency_collector collect --out ../data_low_latency --duration-sec 320

# 6) Remplacer EPOCH par la valeur affichée (slug btc-updown-5m-EPOCH) ou le nom du dossier window_EPOCH
./target/release/low_latency_collector validate --out ../data_low_latency --window-epoch EPOCH
```

---

## 9. Fichiers de documentation associés

| Fichier | Contenu |
|---------|---------|
| [`LOW_LATENCY_COLLECTOR.md`](LOW_LATENCY_COLLECTOR.md) | Architecture, schéma des sorties NDJSON, workflow benchmark |
| [`GUIDE.md`](GUIDE.md) | Viewer TUI `polymarket-viewer` + section collecteur |
| [`../LOW_LATENCY_BASELINE.md`](../LOW_LATENCY_BASELINE.md) | Rappel des limites du collecteur Python snapshot (racine du repo) |
| [`../README.md`](../README.md) | Vue d’ensemble repo Python + pointeur vers le collecteur Rust |

---

## 10. Dépannage rapide

- **`cargo: command not found`** : `source ~/.cargo/env` ou rouvrir le terminal après `rustup`.
- **Erreur TLS / certificats** : installer `ca-certificates` (voir messages de `install.sh`).
- **403 sur Gamma** : le client HTTP du collecteur envoie un User-Agent type navigateur ; en test manuel avec `curl`, utilisez `-A "Mozilla/5.0"`.
- **Pas de dossier `window_*`** : vérifiez `--out`, attendez quelques secondes après le démarrage, ou allongez `--duration-sec`.

---

## 11. Où est le « code complet » ?

Le collecteur est **un seul fichier Rust** :

- **Chemin** : `Viewer rust rtds/src/bin/low_latency_collector.rs`

Pour l’ouvrir ou le copier :

```bash
wc -l "/chemin/vers/Viewer rust rtds/src/bin/low_latency_collector.rs"
less "/chemin/vers/Viewer rust rtds/src/bin/low_latency_collector.rs"
```

Il n’existe pas de fichier de configuration séparé : les URLs et le symbole Chainlink sont les **constantes** listées en section 1. Les sous-commandes et options sont définies dans **`main`** et **`parse_collect_cfg` / `parse_validate_cfg`** dans ce même fichier.
