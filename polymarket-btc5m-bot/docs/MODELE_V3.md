# Modèle v3 — « le trader humain outillé » (06/07/2026 soir)

## Généalogie (chaque version corrige un défaut mesuré)

| Version | Probabilité | Défaut mesuré qui a motivé la suivante |
|---|---|---|
| v1 | Gaussienne sur z | annonce 0,995 quand la réalité est 0,75 (kurtosis 238) |
| v2 | Student-t(ν=2) + table de calibration sur bacs (z × τ) | z est pollué par le bruit d'estimation de σ : à 20 $ d'écart, z=2,9 ne veut rien dire — tous les états perdants vivaient là |
| **v3** | **Student-t (prior) + table de calibration sur bacs (ÉCART EN DOLLARS × τ)** + mode certitude | — (en validation) |

## Les trois idées de la v3

**1. La variable du trader humain.** L'écart |spot − strike| en dollars
bruts remplace z comme clef de la table de calibration empirique
(`DIST_BINS` : 10/20/35/50/75/100/150 $ × `TAU_BINS` : 3-240 s). C'est la
variable robuste : elle ne dépend d'aucune estimation. La loi Student-t
reste le prior des bacs vides ; le postérieur Beta-binomial prend le
dessus quand les observations s'accumulent (auto-calibration en ligne,
persistée, mise à jour à chaque fenêtre réglée).

**2. Le mode certitude** — l'opportunité identifiée par l'utilisateur et
confirmée par l'étude 4 : écart ≥ 50 $ ET τ ≤ 60 s. Le carnet vend alors
le favori à 0,93-0,96 alors que P(win) mesurée = 0,96-1,00 (15 entrées
simulées, 15 gagnantes, +221 $ sur le corpus propre). Dans ce mode, le
plafond de prix passe de 0,85 à `max_entry_certitude` (0,96) et l'edge
minimal descend à 2 points nets après frais : gains unitaires faibles
(~10-15 $ par trade de 250 $), probabilité haute. Hors de ce mode, les
règles prudentes v2 s'appliquent inchangées.

**3. Vers l'avance informationnelle.** Mesure décisive de l'étude 5 : le
relais spot de Polymarket (`crypto_prices`) est EN RETARD de ~5 s sur
l'oracle — inutilisable comme source d'avance. Un module de capture
DIRECTE Binance (`pm-acquisition::binance`, flux btcusdt@trade archivé
verbatim, zéro décision branchée) alimente la prochaine étude lead-lag.
Si l'avance réelle est confirmée, elle deviendra la confirmation d'entrée
du mode certitude (et la clef d'un maker viable — rebates 20 %, zéro
frais maker).

## Garde-fous (hérités, jamais assouplis)

Flux stale/strike douteux/spot vieux ⇒ aucun trade ; écart < 50 $ ⇒ aucun
trade ; profondeur réelle vérifiée ; quart de Kelly plafonné ; frais taker
réels (0,07·p(1−p)) dans l'EV ; RiskGate (kill-switch sur ✗, plafonds) ;
détecteur de silence sur les vrais ticks (correctif PONG du 06/07).

## Ce que la v3 ne prétend PAS

L'edge du mode certitude repose sur 15 trades simulés / ~70 fenêtres :
c'est une hypothèse forte, pas une preuve. La table l'affinera ou
l'éteindra d'elle-même (si les bacs 50 $+ se dégradent, l'EV repasse sous
le seuil et le bot s'abstient). Décision d'argent réel : après un dry run
long et un edge qui survit à ≥ 50 trades.

---

# Addendum v4 — « la frontière » (06/07/2026, nuit)

La v4 remplace le mode certitude par LA FRONTIÈRE mesurée (étude 6,
balayage de 90 variantes, bornes de Wilson) et intègre les résultats
négatifs autant que positifs :

- **Entrée** : écart ≥ 70 $ ET τ ≤ 120 s ET prix ≤ 0,98 ET
  EV_calibrée − frais ≥ 1 point. Seule règle prouvée à 90 % de confiance
  (34/34, +521 $/13 h simulés ; 2,6 trades/h).
- **Curseur de confiance** (config) : prudent 0,96 / standard 0,98 /
  agressif dist 40 $ — fréquence × certitude au choix.
- **Rejetés par la mesure** : frontière lissée c·√τ (dilue dans la zone
  mixte, aucune variante prouvée) ; stop-loss dynamique (+244 → +118 $ :
  vendre les retournements verrouille des pertes que le règlement
  récupère) ; multi-entrées. Les positions portent au règlement.
- **Validation marche avant intégrale (table froide)** : **+83,22 $ /
  9 entrées / 164 fenêtres** — premier corpus positif du projet. Le live
  démarre avec la table mûre (188 fenêtres).
- Moteur ÉVÉNEMENTIEL (décision à chaque tick, −125 ms en moyenne),
  exactitude prouvée par A/B parallèle (733/733 ticks, strikes au
  dix-millième).
