# La ligne d'efficience du trader humain — étude 6 (06/07/2026)

Corpus : 158 fenêtres à carnet complet ≈ 13,2 h de marché.
Méthode : simulation trade par trade (1 entrée max/fenêtre, première
seconde qualifiante, frais taker réels 0,07·p(1−p), ½ tick de slippage),
borne basse de Wilson à 90 % sur chaque variante. Script :
`analysis/etude6b_balayage.py` (60 variantes écart × horizon × prix).

## Le moment parfait (résultat central)

> **Écart |spot − strike| ≥ 70 $ avec ≤ 2 minutes restantes : acheter le
> favori jusqu'à 0,96-0,98.**

| Variante | Trades | Fréquence | Réussite | PnL (13 h) | $/trade | Preuve 90 % |
|---|---|---|---|---|---|---|
| ≥70 $, ≤120 s, prix ≤0,98 | 34 | **2,6/h** | 100 % | **+521 $** | +15,3 | ✅ P_lo 0,954 > seuil 0,949 |
| ≥70 $, ≤120 s, prix ≤0,96 | 23 | 1,7/h | 100 % | +490 $ | +21,3 | ✅ P_lo 0,934 > seuil 0,929 |
| ≥50 $, ≤60 s, prix ≤0,96 (ex-certitude) | 16 | 1,2/h | 100 % | +268 $ | +16,8 | ✗ (échantillon trop petit) |
| ≥30 $, ≤60 s, prix ≤0,96 (agressif) | 57 | 4,3/h | 91 % | +285 $ | +5,0 | ✗ |

## La frontière (forme de la ligne d'efficience)

L'écart nécessaire CROÎT avec l'horizon — cohérent avec la diffusion
(≈ ∝ √τ) mais avec une prime de prudence :

| Horizon τ | Écart minimal efficace | Au-dessous |
|---|---|---|
| ≤ 60 s | ~30-40 $ (point estimate positif) | perdant |
| 60-120 s | ~70 $ (prouvé) | mixte/perdant |
| > 120-200 s | ≥ 90-130 $ et encore fragile | perdant |
| > 200 s | aucun écart réaliste ne suffit | le marché price mieux |

La bande 15-40 $ à τ > 60 s est le cimetière : c'est là que vivaient
toutes les pertes historiques du bot.

## Le curseur de confiance (fréquence ↔ certitude)

- **Prudent** (prouvé à 90 %) : ≥70 $/≤120 s/≤0,96 → 1,7 trade/h, +21 $/trade
- **Standard** (prouvé à 90 %) : ≥70 $/≤120 s/≤0,98 → 2,6 trades/h, +15 $/trade
- **Agressif** (estimation ponctuelle positive, non prouvée) :
  ≥30 $/≤60 s/≤0,98 → 5,6 trades/h, +4 $/trade
- En dessous : ~10 trades/h mais PnL négatif — la sur-fréquence détruit.

## Honnêteté statistique

- Le « prouvé à 90 % » du vainqueur est MINCE (marge +0,005) et issu d'un
  balayage de 60 variantes (risque de sélection) — mais la région entière
  ≥50-90 $ × ≤120 s est positive et cohérente, pas une cellule chanceuse
  isolée.
- 13 h de marché d'un week-end. Le régime de semaine peut différer.
  La table de calibration en ligne continuera de mesurer.
