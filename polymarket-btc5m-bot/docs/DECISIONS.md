# Journal des décisions — chaque choix avec la mesure qui l'a motivé

Principe méthodologique central du projet, appris à nos dépens :
**l'intention du prompt et l'intuition priment moins que la mesure.**
Plusieurs demandes initiales pointaient ailleurs que les données ; c'est
la mesure qui a tranché, à chaque fois. Un successeur (humain ou modèle)
doit vérifier les hypothèses contre les archives AVANT d'optimiser dans
leur direction — y compris les hypothèses de ce document si de nouvelles
données les contredisent.

## Décisions actives (et leurs preuves)

| Décision | Mesure qui la fonde | Où |
|---|---|---|
| Strike = dernier tick Chainlink ≤ T0 | 5/5 exact vs UI (0,00 $ d'écart), ~300 fenêtres confidence 1.0 | VALIDATION_LIVE.md |
| Loi Student-t (ν=2) et non gaussienne | kurtosis 238 ; P(\|r\|>5σ) ×10⁴ gaussien ; gaussienne annonce 0,995 quand la réalité fait 0,75 | ETUDE_MODELE.md |
| Table de calibration en ÉCART $ × τ (pas z) | tous les états perdants < 50 $ d'écart ; z gonflé par le bruit de σ (perte du 06/07 06:09 : z=2,8 sur 13 s / petit écart) | etude4, LIGNE_EFFICIENCE.md |
| Entrée « frontière » : ≥70 $, ≤120 s, ≤0,98 | seule règle > borne Wilson 90 % : 34/34, +521 $/13,2 h | etude6b, LIGNE_EFFICIENCE.md |
| Frais taker 0,07·p(1−p) dans l'EV | barème officiel crypto (docs Polymarket) ≈ 2 % du notional | trading/fees |
| Positions portées au règlement (pas de stop) | stop testé : +244 → +118 $ (vendre les retournements verrouille des pertes que le règlement récupère) | etude7 |
| Décision événementielle (au tick) | minuterie 250 ms = jusqu'à 250 ms perdues ; A/B parallèle : 733/733 ticks identiques | AUDIT_VITESSE.md |
| Silence des flux mesuré sur les vrais ticks | connexion à moitié morte indétectable via PONG (45 min de strike figé le 06/07 17 h) | commit du correctif |
| Kill-switch sur contradiction ✗ + RiskGate | défense en profondeur ; refus vérifiés au comportement | MVP_REEL.md |

## Idées RÉFUTÉES par la mesure (ne pas les réintroduire sans nouvelles preuves)

| Idée séduisante | Verdict mesuré |
|---|---|
| Taker « valeur » (z fort + ask bas = carnet en retard) | l'ask EST la probabilité calibrée ; edge négatif partout conditionné à l'entrée ; −228 $ attribuables sur le corpus. Le carnet n'est pas en retard : NOUS le sommes (oracle +1,4 s, MM sur le spot) |
| Le marché se trompe / le modèle sait mieux | Brier marché 0,17 < modèle 0,22 ; à z égal, p_mkt stratifie parfaitement les issues |
| Acheter le favori très cher, souvent (loi des grands nombres) | P(win) < prix+frais à TOUS les niveaux 0,90-0,99 ; 1 000 trades ⇒ −1 571 $ ± 1 565 (la répétition certifie le signe, elle ne le change pas) |
| Latence exploitable après un saut d'oracle | le carnet a bougé AVANT notre réception (médiane 0,000 à +250 ms) |
| Arbitrage de parité Up+Down < 1 | jamais observé exploitable (somme ≥ 1,001, méd. 1,010) |
| Relais spot RTDS comme source en avance | il RETARDE de ~5 s sur l'oracle (corr 0,977 à lag −5 s) |
| Frontière lissée c·√τ | dilue dans la zone mixte ; aucune variante ne passe la borne de Wilson |
| Stop-loss dynamique | détruit +126 $ sur le corpus |
| Maker naïf (quotes au fair ± marge) | sélection adverse : 108/108 configs perdantes |
| Drift extrapolé sans plafond | fabrique des certitudes (z=−5,6 sur 12 $ d'écart → −258 $) ; plafonné à 2 z |

## Fragilités assumées (à re-tester avec plus de données)

- La preuve de la frontière est MINCE (marge Wilson +0,005) et issue d'un
  balayage de ~90 variantes (risque de sélection) ; 13 h de week-end
  seulement. La table auto-apprise arbitrera en continu.
- Poche jamais validée hors échantillon sur un régime de semaine/annonce.
- Le corpus walk-forward positif (+83 $) porte sur 9 entrées.
