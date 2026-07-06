# VISION — trajectoire long terme & questions de recherche ouvertes

Document de passation : pensé pour qu'une nouvelle session (Claude Code /
Opus 4.8, contexte 1M, fort en agentic long-horizon) puisse planifier à
plusieurs semaines SANS refaire le chemin déjà parcouru — et sans se
laisser enfermer par la lettre des prompts : le §« méthode » de
docs/DECISIONS.md prime.

## Où va le projet (par ordre de dépendance)

```
[FAIT] plateforme données/exécution/risque fiable et auditée
[FAIT] modèle v4 « frontière » — premier walk-forward positif (+83 $)
[ICI]  micro-test réel ≤ 20 $ (GO utilisateur requis) → écart paper↔réel
  │
  ├─ A. Consolidation statistique de la frontière (passive, continue)
  ├─ B. Lead-lag Binance direct → l'avance informationnelle
  └─ C. Maker v2 nourri par B (rebates 20 %, frais 0) — le vrai gisement
```

## Questions de recherche OUVERTES (avec l'état des preuves)

### A. La frontière tient-elle hors échantillon ?
- Preuve actuelle : 34/34 mais borne Wilson mince (+0,005), 13 h de
  week-end. **Critère de passage** : ≥ 50 trades live/paper avec
  P(win) − seuil > 0 sur la borne basse 90 %. La table auto-apprise
  (data_v2/calibration.json) accumule toute seule — NE PAS réinitialiser.
- Piste : hold-out temporel strict (calibrer semaine N, mesurer N+1).

### B. Lead-lag Binance direct → oracle (l'avance informationnelle)
- Fait : le carnet reprice AVANT notre réception de l'oracle (donc via le
  spot) ; notre capture Binance directe est à 234 ms (p50) ; le relais
  RTDS est disqualifié (−5 s).
- Question : de COMBIEN le spot direct précède-t-il les prints de
  l'oracle, et cette avance suffit-elle à prédire le print FINAL dans les
  10-30 dernières secondes mieux que le carnet ne price ?
- Données : stream "binance" dans les journaux depuis le 06/07 soir
  (63 960 events/h). Étendre analysis/etude5 avec l'horodatage `E` de
  Binance vs `payload.timestamp` de l'oracle, PUIS conditionner la
  frontière sur la confirmation spot. Si l'avance est réelle → l'entrée
  frontière devient un trade à information anticipée (cf. mesure : à
  0,985 il faut 98,6 % — quelques secondes d'avance suffisent à basculer).

### C. Maker v2 (le gisement structurel)
- Pourquoi y croire : les makers paient 0 frais + rebates 20 % (barème
  crypto) — le taker subventionne. Pourquoi ce n'est pas fait : sélection
  adverse mesurée (108/108 configs perdantes) tant que notre fair est en
  retard. Débloqué par B (fair au rythme des incumbents) + quotes
  uniquement tôt dans la fenêtre + inventaire borné.

### D. Divers mesurables
- Sizing : Kelly sur p_borne_basse plutôt que p ponctuelle.
- Régimes : la frontière par régime de vol (calme/tempête) — la table
  $×τ pourrait gagner une 3ᵉ dimension quand l'effectif le permettra.
- WSS user + réconciliation automatique fills réels↔paper (P2 de
  MVP_REEL.md) dès que le volume réel le justifie.

## Ce qui est DÉFINITIVEMENT établi (ne pas re-dériver)

Voir docs/DECISIONS.md — en particulier : le taker « valeur » est mort,
le marché est mieux calibré que tout modèle sans avance informationnelle,
et l'oracle nous parvient 1,4 s en retard (incompressible).

## Contrats avec l'utilisateur (clement.bondaz@imt-bs.eu)

1. **Rien ne se lance sans son GO explicite** (dry run compris).
2. Micro-test réel plafonné à 20 $ (RiskGate), quelques heures.
3. Il pense en trader : traduire les critères statistiques en langage
   trading (écart $, temps restant, prix d'entrée) — c'est SA lecture
   (écart × τ) qui a trouvé la frontière que les z-scores rataient.
4. Honnêteté totale sur les résultats négatifs : il les préfère aux
   promesses.
