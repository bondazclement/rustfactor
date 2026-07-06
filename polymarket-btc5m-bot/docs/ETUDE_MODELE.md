# Étude quantitative du modèle — 06/07/2026

Corpus : 94 journaux (04-06/07), 89 598 ticks Chainlink (~25 h), 338
fenêtres découvertes, 295 fenêtres à couverture complète, 39 067 instants
avec carnet joint (138 fenêtres). Scripts : `analysis/*.py`, données
compactes : `analysis/out/`.

## 1. Le processus de prix n'est PAS une diffusion gaussienne

- Rendements 1 s : σ ≈ 0,34 bp, **kurtosis excédentaire = 238**.
- P(|r| > 5σ) observée = 5,8×10⁻³ soit **10 000× le taux gaussien** ;
  les sauts à 8σ arrivent ~150×/jour. C'est un processus à sauts
  (mécanique Chainlink : heartbeat ~1 s + agrégation par seuils).
- Autocorrélation positive aux échelles 1-15 s (+0,24 à 1 s), nulle à 60 s.
- Ajustement Student-t sur r_1s : **ν ≈ 0,5** (queues énormes).
  P(tenir un z de 3) : gaussien 0,999 → t(ν=0,5) : 0,81.

## 2. Le modèle gaussien actuel est massivement surconfiant

Calibration mesurée sur 84 034 instants (p prédit vs fréquence réalisée) :

| p annoncé | réalité (tous τ) | réalité (τ≤120 s) |
|---|---|---|
| 0,95-0,99 | 0,667 | 0,741 |
| 0,995-0,999 | 0,755 | 0,856 |
| > 0,999 | 0,903 | 0,944 |

Brier score : **modèle 0,223 — marché 0,172**. Le carnet price MIEUX que
notre modèle, systématiquement.

## 3. Conditionné à nos moments d'entrée, l'edge est NÉGATIF

P(gagner) mesurée quand z ≥ seuil ET l'ask du côté favori est encore bas
(= la définition même de nos entrées) :

| Condition d'entrée | n | P(win) réelle | ask moyen | edge |
|---|---|---|---|---|
| z∈[2;2,5), ask∈(0,65;0,75] | 116 | 0,716 | 0,717 | **−0,002** |
| z∈[2,5;3), ask∈(0,75;0,85] | 73 | 0,493 | 0,815 | **−0,32** |
| z∈[2,5;3), ask∈(0,85;0,95] | 738 | 0,917 | 0,922 | **−0,004** |

**L'ask EST la probabilité bien calibrée.** Un carnet qui « vend encore
bas » après un mouvement n'est pas en retard : il price un risque de
retournement réel que le z ne voit pas. À z égal, la prob implicite du
marché stratifie parfaitement les issues (p_mkt 0,4-0,6 → 52 % ; 0,9-1,0
→ 98 %) : conditionnellement au prix marché, z n'apporte RIEN.

Simulation sur 138 fenêtres : la règle de prod (z≥2,5, ask≤0,75) fait
−1 178 $/12 trades. Toutes les variantes testées (EV sur table empirique,
mix marché, Student-t) restent négatives. Les +330/+489 $ des premiers
backtests étaient un artefact de petit échantillon (3-6 trades, in-sample).

## 4. Les gisements microstructure sont fermés (mesures)

- **Latence** : nos ticks oracle arrivent 1,3-1,7 s après leur timestamp
  source ; après un saut >4σ, l'ask du côté du saut ne bouge PLUS
  (médiane 0,000 à +250 ms comme à +5 s) — les MM ont repricé AVANT
  notre réception (ils lisent les bourses spot, qui précèdent Chainlink).
  Nous sommes structurellement derniers dans la chaîne d'information.
- **Parité** : ask_up + ask_down ≥ 1,001 (p1) ; médiane 1,010 ; jamais
  < 0,97 de façon exploitable. L'overround ~1 % est la marge du MM.
- **Continuation post-saut** : réelle sur l'oracle (74 % à 5 s) mais déjà
  dans les prix du carnet.
- **Frais réels taker (catégorie Crypto)** : `0,07 × p(1−p)` par share
  ≈ 1,5 ¢ à p∼0,7, soit ~2 % du notional — le paper les sous-comptait.

## 5. Conclusion et décision de design

**Le signal « z du prix contre le strike » n'a pas d'edge taker sur ce
marché.** Même une probabilité parfaite (= celle du marché) est perdante
en croisant le spread + l'overround + les frais. Le problème n'était pas
la calibration du modèle : c'est l'absence d'avantage informationnel.

Décisions :
1. **Moteur de probabilité honnête** (implémenté) : CDF Student-t (ν
   configurable) + **table de calibration empirique auto-apprise**
   (buckets z×τ, lissage bayésien Beta-binomial, persistée, mise à jour
   en ligne à chaque fenêtre réglée). Le bot connaît désormais ses vraies
   probabilités — vérifiable au Brier.
2. **Taker piloté par l'EV réelle** : entrée seulement si
   p_honnête − prix − frais(p) ≥ marge. Conséquence assumée : avec les
   prix actuels du carnet, il ne tradera presque jamais — c'est la
   décision correcte (ne pas payer pour du −EV).
3. **Chasse au vrai edge** : capture du flux spot Binance (RTDS
   `crypto_prices`) en parallèle de Chainlink pour mesurer le lead-lag
   et évaluer un modèle « source en avance » (seul avantage
   informationnel accessible). Décision d'activation après données.
4. Maker : reste fermé (sélection adverse démontrée) tant qu'un
   re-design complet n'est pas validé sur données.
