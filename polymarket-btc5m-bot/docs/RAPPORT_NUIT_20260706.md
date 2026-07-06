# Rapport du dry run local — nuit du 05 au 06/07/2026

Campagne `pm-ctl demarrer --max-entry 0.75 --cible 200 --tranches 500`,
lancée le 05/07 à 20:43 UTC, arrêtée proprement le 06/07 à 07:15 UTC.
Config identique sur toute la nuit (défauts calibrés + plafond 0,75) :
`min_abs_z=2.5` (×2 en début de fenêtre), `min_edge=0.06`, quart de Kelly,
`max_notional=250`, drift plafonné à 2 z, `min_tau_s=3`, maker désactivé.

## Cycles (13 tranches de ~50 min)

| Cycle | Début UTC | Fenêtres réglées | Entrées | PnL tranche | Résolutions |
|---|---|---|---|---|---|
| 1 | 20:43 | 9 | 1 | **+76,60** | 9 ✓ / 0 ✗ |
| 2 | 21:33 | 9 | 1 | **−253,76** | 9 ✓ / 0 ✗ |
| 3 | 22:23 | 6* | 1 | **+80,83** | 6 ✓ / 0 ✗ |
| 4 | 23:13 | 9 | 0 | 0 | 9 ✓ / 0 ✗ |
| 5 | 00:04 | 8 | 1 | (+95,55 non comptés — voir note) | 8 ✓ / 0 ✗ |
| 6–11 | 00:54 → 05:55 | 52 | 0 | 0 | 52 ✓ / 0 ✗ |
| 12 | 05:55 | 8 | 1 | **−252,24** | 8 ✓ / 0 ✗ |
| 13 | 06:45 | (interrompu par l'arrêt) | 0 | 0 | — |
| **Total** | 10,5 h | **≈ 101 fenêtres** | **5** | **−253,02** (avec reconstruction) | **101 ✓ / 0 ✗** |

*Cycle 3 : moins de règlements car démarrage lent du flux (reconnexions), sans impact.

Qualité de la chaîne de données : **zéro contradiction sur 101 résolutions
officielles**, strike gelé à confidence 1.000 / gap 0 ms sur toutes les
fenêtres pleines. Volume : ~13 Go bruts → 1,1 Go archivés (zstd).

## Les 5 trades de la nuit (détail complet)

Sortie taker = règlement (1,00 si gagné, 0,00 si perdu). Taille ≈ quart de
Kelly plafonné à 250 $ de notional.

| # | Heure UTC | Fenêtre | τ restant | Côté | Prix moyen | Taille | z | p modèle | Issue | PnL |
|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 21:29:10 | 1783286700 | 50 s | UP | 0,764 | ~327 | +2,50 | 0,994 | Up ✓ | **+76,60** |
| 2 | 22:03:59 | 1783288800 | 60 s | UP | 0,719 | ~348 | +2,76 | 0,997 | Down ✗ | **−253,76** |
| 3 | 22:53:21 | 1783291800 | 98 s | UP | 0,757 | ~330 | +3,12 | 0,999 | Up ✓ | **+80,83** |
| 4 | 00:53:15 | 1783299000 | 104 s | DOWN | 0,727 | 350 | −2,96 | 0,998 | Down ✓ | **+95,55** (reconstruit) |
| 5 | 06:09:46 | 1783317900 | 13 s | DOWN | 0,599 | ~420 | −2,79 | 0,997 | Up ✗ | **−252,24** |

**Note trade #4** : entré à 104 s de la fin, la tranche a été tuée 53 s
avant le règlement → position jamais réglée dans les logs (`pnl_tranche=+0.00`
malgré `entrées=1`). Issue vérifiée via l'API Gamma (résolution officielle :
Down) → gain reconstruit +95,55 $. **Défaut à corriger** : la boucle doit
attendre le règlement des positions ouvertes avant de tuer une tranche.

## Lecture

- Cumul ère calibrée (drift plafonné, 7 trades : 5 locaux + 2 cloud) :
  4 ✓ / 3 ✗, **−413,47 $**. Gains ~+76…+95 (prix d'entrée ~0,72 → gain
  ≈ 38 % du notional), pertes ~−253 (mise entière) : à ces prix d'entrée,
  il faut ~72 % de réussite pour l'équilibre — on mesure ~57 %.
- Le modèle annonce p ≥ 0,994 sur CHAQUE entrée et perd 3 fois sur 7 :
  **le modèle est massivement surconfiant** — c'est le problème central,
  confirmé par l'étude quantitative (docs/ETUDE_MODELE.md).
- Mode d'échec « τ court » confirmé : τ=11 s et 13 s → 2/2 perdants.
  Le troisième perdant (τ=60 s, z=2,76) montre que le problème dépasse
  les toutes dernières secondes.
