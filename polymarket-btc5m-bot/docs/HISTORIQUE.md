# Historique du projet — des prototypes au modèle v4

Ce document remplace les anciennes versions du dépôt (supprimées pour
alléger le déploiement : ~763 Mo d'artefacts de build). Il conserve la
mémoire des itérations qui ont mené au bot actuel — pourquoi chaque
version a existé, ce qu'elle a appris, pourquoi elle a été dépassée. Les
décisions techniques chiffrées sont dans `DECISIONS.md` ; ici, c'est la
lignée.

## Le problème, invariant depuis le début

Marché Polymarket **« Bitcoin Up or Down »** toutes les 5 minutes
(`btc-updown-5m-<epoch>`). À l'ouverture d'une fenêtre, un « price to
beat » (strike) est figé ; 5 minutes plus tard le marché résout **Up** si
le prix Chainlink BTC/USD est au-dessus, **Down** sinon. Deux jetons (Up,
Down) s'échangent entre 0 et 1 $ sur un carnet d'ordres. Objectif :
décider quand acheter l'un des deux avec une espérance positive.

La difficulté centrale, découverte très tôt et jamais démentie : **la
résolution dépend EXCLUSIVEMENT du flux Chainlink natif de Polymarket**
(RTDS `crypto_prices_chainlink`), pas des bourses spot. Toute source
externe (Coinbase, Binance) pour la résolution ou la volatilité fabrique
de faux signaux. C'est la règle d'or du projet.

## Version 1 — collecteur Python (« version avancée en python »)

Premier prototype : un collecteur Python qui prenait des snapshots
(~500 ms) du marché dans un CSV par fenêtre.

**Ce qu'il a apporté**
- La preuve que le strike se reconstruit depuis le RTDS Chainlink autour
  de `eventStartTime`, avec un score de confiance, sans dépendre du
  Candlestick de l'UI.
- Un « price to beat » en moyenne **plus exact** que les versions
  suivantes (relevé de terrain : $80,714.93 enregistré vs $80,714.87 réel
  sur la fenêtre 1778343900 — biais de +0,06 $ dû à l'interpolation).
- Une interface de contrôle runtime (staging/validation des corrections
  de strike sans arrêter le bot).

**Pourquoi il a été dépassé**
- **Latence RTDS de 5-7 s** entre chaque collecte : rédhibitoire pour un
  marché de 5 minutes où les dernières secondes décident tout.
- Il affichait le cours **Coinbase** et une volatilité Coinbase — sans
  aucun lien avec la résolution Polymarket. Joli, mais trompeur : c'est
  précisément le genre de source externe qui a causé des faux trades.
- Python + WebSockets : trop de couches pour viser la milliseconde.

## Version 2 — « Rustector_btc_5mn_1 » (prototype Rust mono-crate)

Réécriture en Rust pour tuer la latence. Un seul crate, une TUI live.

**Ce qu'il a apporté**
- Collecte RTDS Chainlink `btc/usd` + CLOB (carnet/trades) en Rust, avec
  enregistrement NDJSON verbatim en parallèle — la latence tombe à
  l'échelle de la centaine de millisecondes.
- Le principe **« archive avant parsing »** : chaque trame réseau
  journalisée brute avant toute interprétation → tout run est rejouable.
- Les jeux de données de référence (`data_low_latency`) qui ont servi aux
  toutes premières analyses de strike.

**Pourquoi il a été dépassé**
- Mono-crate, orienté collecte/affichage : pas d'architecture pour porter
  un modèle de décision, un backtest partagé avec le live, une exécution
  réelle sous garde-fous.
- Aucune stratégie de trading, aucun paper broker, aucune calibration.

## Version 3 — le workspace actuel (`polymarket-btc5m-bot`)

Refonte complète en workspace multi-crates (voir `ARCHITECTURE.md`). Elle
hérite des leçons des deux précédentes : latence Rust de la v2, exactitude
du strike de la v1, règle d'or « flux natif uniquement ». Puis elle ajoute
ce qui manquait — un modèle, un backtest fidèle au live, une exécution
réelle bardée de garde-fous, une interface.

L'évolution du **modèle de décision** à l'intérieur de cette version est
une histoire à part entière, résumée ici et détaillée dans `DECISIONS.md`,
`ETUDE_MODELE.md`, `LIGNE_EFFICIENCE.md` et `MODELE_V3.md` :

| Modèle | Idée | Ce que la mesure a dit |
|---|---|---|
| Gaussien sur z | diffusion log-normale classique | surconfiant (annonce 0,995 quand la réalité fait 0,75 ; kurtosis mesurée 238) |
| Student-t + calibration sur z | queues épaisses + table apprise | mieux, mais z pollué par le bruit d'estimation de la volatilité |
| **v4 « la frontière »** | table de calibration en **écart-dollars × temps restant**, entrée uniquement dans la zone prouvée (≥ 70 $, ≤ 120 s, ≤ 0,98) | seule règle positive à 90 % de confiance ; premier corpus walk-forward positif |

Le tournant méthodologique : c'est en pensant **comme un trader humain**
(« gros écart au strike + peu de temps pour revenir ») plutôt qu'en
z-scores normalisés que la zone rentable est apparue. Les données ont
tranché contre plusieurs intuitions séduisantes (le taker « valeur », le
stop-loss, l'arbitrage de parité…) — toutes consignées dans `DECISIONS.md`
pour ne pas les re-tester naïvement.

## Ce qui reste des anciennes versions

Rien dans le code : elles sont supprimées. Leur héritage est immatériel —
les trois règles d'or (flux natif, archive avant parsing, garde-fous non
négociables), la latence Rust, et une longue liste d'impasses déjà
explorées. Les données de référence historiques, elles, ont été rejouées
et digérées dans la table de calibration et les études `analysis/`.
