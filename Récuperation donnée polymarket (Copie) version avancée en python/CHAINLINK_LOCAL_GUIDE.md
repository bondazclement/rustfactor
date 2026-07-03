# Chainlink Local et ce bot Python (guide complet)

> Note version actuelle du bot: l'integration **Chainlink Data Streams** a ete retiree.
> Le bot utilise uniquement les flux RTDS Polymarket (`crypto_prices_chainlink` et `crypto_prices`).
> Ce document reste une archive technique sur Chainlink Local et ne decrit plus le chemin de production du bot.

Ce document explique:

1. ce qu'est vraiment `@chainlink/local`,
2. pourquoi `chainlink --help` ne fonctionne pas apres `npm install @chainlink/local`,
3. comment l'installer correctement selon le cas (Hardhat / Foundry / Remix),
4. ce que tu peux faire avec pour ce projet de bot Python,
5. ce que tu ne peux pas faire (notamment pour obtenir des credentials Data Streams / Candlestick).

---

## TL;DR

- `@chainlink/local` n'installe pas une commande shell `chainlink`.
- C'est une librairie de simulation locale (contrats + scripts) pour dev smart contract.
- Donc `chainlink --help` -> `commande non trouvee` est normal.
- Pour ce bot Python:
  - `CHAINLINK_CANDLE_USER` / `CHAINLINK_CANDLE_KEY` et `CHAINLINK_DS_*` ne viennent pas de `chainlink-local`.
  - Ils viennent d'un acces Data Streams/Candlestick fourni par Chainlink (service distant).

---

## 1) Pourquoi `chainlink --help` ne marche pas

Tu as lance:

```bash
npm install @chainlink/local@0.2.3
chainlink --help
```

Le package `@chainlink/local` ne declare pas de champ `bin` (donc pas de CLI global `chainlink`).
Il installe des fichiers Solidity/scripts dans `node_modules/@chainlink/local`.

Tu peux verifier:

```bash
cat node_modules/@chainlink/local/package.json
```

Tu verras des champs comme `files`, `scripts`, `dependencies`, mais pas de binaire CLI expose.

---

## 2) Ce qu'est Chainlink Local

Chainlink Local est un outil de simulation locale pour developper et tester:

- CCIP local (`CCIPLocalSimulator`, etc.),
- mocks Data Feeds (`MockV3Aggregator`, `MockOffchainAggregator`),
- tokens utilitaires (`LinkToken`, `WETH9`).

Ce n'est pas un endpoint API Data Streams local qui te donne des credentials WS/HMAC pour le bot.

References:

- [Chainlink Local overview](https://docs.chain.link/chainlink-local)
- [Chainlink Local API reference v0.2.3](https://docs.chain.link/chainlink-local/api-reference/v0.2.3)
- [smartcontractkit/documentation](https://github.com/smartcontractkit/documentation)

---

## 3) Prerequis (Fedora) pour tester Chainlink Local

### Outils minimaux

```bash
sudo dnf update -y
sudo dnf install -y git curl
```

### Node.js + npm (pour Hardhat)

```bash
sudo dnf module install -y nodejs:20
node -v
npm -v
```

### (Optionnel) Foundry (si tu veux la voie Solidity/Anvil)

```bash
curl -L https://foundry.paradigm.xyz | bash
source ~/.bashrc
foundryup
forge --version
anvil --version
```

---

## 4) Installation de A a Z: mode Hardhat + @chainlink/local

Ce mode est le plus simple pour verifier que Chainlink Local est bien installe.

```bash
cd "/tmp"
mkdir -p chainlink-local-hardhat-demo
cd chainlink-local-hardhat-demo

npm init -y
npm install --save-dev hardhat
npx hardhat init
```

Puis installer Chainlink Local:

```bash
npm install @chainlink/local@0.2.3
```

Verifier installation:

```bash
npm list @chainlink/local
ls node_modules/@chainlink/local
```

Important:

- Tu n'auras toujours pas de commande `chainlink`.
- Tu utilises `@chainlink/local` par import dans scripts/tests Solidity/Hardhat.

---

## 5) Installation de A a Z: mode Foundry

```bash
cd "/tmp"
forge init chainlink-local-foundry-demo
cd chainlink-local-foundry-demo

forge install smartcontractkit/chainlink-local@7d8b2f888e1f10c8841ccd9e0f4af0f5baf11dab
```

Configurer les remappings (un des deux):

- `remappings.txt`
- ou `foundry.toml`

Avec:

```text
@chainlink/local/=lib/chainlink-local/
```

Build:

```bash
forge build
```

---

## 6) Usage minimal (exemple conceptuel)

Dans un test Solidity, tu importes:

- `CCIPLocalSimulator.sol` pour CCIP local,
- `MockV3Aggregator.sol` pour feed mock.

Exemple d'import:

```solidity
import {CCIPLocalSimulator} from "@chainlink/local/src/ccip/CCIPLocalSimulator.sol";
```

L'objectif est de simuler un environnement blockchain local pour tes contrats.

---

## 7) Ce que Chainlink Local n'apporte pas pour ce bot

Ton bot actuel est Python et consomme:

- Polymarket RTDS,
- eventuellement des overrides manuels du strike.

Chainlink Local:

- ne fournit pas les flux RTDS Polymarket du bot.

Donc installer `@chainlink/local` ne remplace pas les flux live utilises en production par ce bot.

---

## 8) Comment l'utiliser utilement avec ce bot (strategie realiste)

### Option A (recommandee)

Utiliser directement les flux Polymarket RTDS et les commandes runtime du bot.

Lancer le bot:

```bash
cd "/home/clementbondaz-sanson/Documents/Projet code/Récuperation donnée polymarket"
./.venv/bin/python -m collector.cli run --strike 66848.13
```

### Option B (dev local pur)

Tu peux utiliser Chainlink Local pour tester des contrats/mocks onchain, mais pas pour reproduire 1:1 les flux RTDS Polymarket du bot.
Pour tester le bot hors ligne, il faut un "mock provider" specifique Python (a developper) qui emule les flux attendus.

---

## 9) Reponse a tes warnings npm

Tes warnings (`inflight`, `rimraf`, `glob`, integrity check git dep) sont des avertissements de dependances transitives.
Ils n'expliquent pas `chainlink: commande non trouvee`.

La vraie cause est l'absence de CLI `chainlink` dans `@chainlink/local`.

---

## 10) Checklist rapide de verification

1. Package installe:

```bash
npm list @chainlink/local
```

2. Fichiers presents:

```bash
ls node_modules/@chainlink/local/src
```

3. Pas de binaire global attendu:

```bash
npm view @chainlink/local bin
```

Si vide/null: c'est confirme, pas de commande `chainlink`.

---

## 11) Liens officiels

- [Chainlink Local](https://docs.chain.link/chainlink-local)
- [Chainlink Local API reference v0.2.3](https://docs.chain.link/chainlink-local/api-reference/v0.2.3)
- [Documentation repo (source docs.chain.link)](https://github.com/smartcontractkit/documentation)

