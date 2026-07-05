# Audit de robustesse — 2026-07-05 (avant dry run long)

Toutes les vérifications ci-dessous ont été exécutées réellement dans
l'environnement distant, commandes et résultats à l'appui.

## 1. Batterie logicielle

| Vérification | Commande | Résultat |
| --- | --- | --- |
| Tests unitaires + intégration | `cargo test --workspace --release` | **72/72 verts** |
| Lints | `cargo clippy --workspace --release` | **0 warning** |
| Formatage | `cargo fmt --check` | conforme |
| Build release | `cargo build --release` | 0 warning |

## 2. Empreinte ressources (mesurée sur les campagnes réelles)

| Métrique | Valeur mesurée |
| --- | --- |
| RAM du bot (RSS) après 15 min | **19 Mo** |
| CPU moyen | **~2 %** d'un cœur |
| Écriture disque (journal brut) | ~1,3 Go/h (~32 Go/jour) |
| Compression zstd des journaux | ratio ~12× (2,6 Go/jour compressé) |

Sur un PC gamer AMD : négligeable. Seul le disque demande de l'attention
sur un dry run de plusieurs jours (voir INSTALLATION_FEDORA.md §Espace).

## 3. Audit « perte de connexion Polymarket » (chaos test réel)

Méthode : blocage TOTAL du trafic sortant du bot (règle pare-feu sur le
port du proxy) pendant **90 secondes**, en pleine campagne live, fenêtre
active — puis rétablissement. Chronologie observée (logs du
2026-07-05, run `camp_20260705T003233`) :

| t (depuis la coupure) | Événement observé |
| --- | --- |
| +6 s | Watchdog : `FEED STALE` sur RTDS **et** CLOB → toute prise de risque coupée |
| +6,4 s | RTDS : tentative de réabonnement (1ʳᵉ ligne de défense) |
| +12,4 s | RTDS : **reconnexion forcée** (connexion déclarée morte) — boucle de reconnexion avec backoff 2→15 s pendant toute la panne |
| +30,7 s | CLOB : **reconnexion forcée** |
| pendant la panne | **0 décision de trading** (vérifié : aucun TAKER/DRY-RUN dans l'intervalle) |
| +92 s (2 s après rétablissement) | CLOB reconnecté, carnet resynchronisé (snapshot `initial_dump`) |
| +151 s | RTDS reconnecté (délai = cumul des backoffs pendant la panne, max 15 s par tentative) |
| fenêtre suivante | Strike gelé avec **confidence 0.000** (tick de T0 manqué pendant la panne) → **fenêtre non tradable**, comportement voulu ; retour à confidence 1.0 dès la fenêtre d'après |
| pendant la panne | Résolution officielle reçue au retour : `CONFIRME ✓` (aucune corruption d'état) |

Verdict : **aucun crash, aucune perte d'état, aucun trade en aveugle,
récupération autonome complète**. Le PnL ne peut pas être affecté par une
coupure : les garde-fous (staleness, confidence du strike) bloquent toute
décision tant que la donnée n'est pas redevenue sûre.

Incidents réels déjà survenus et couverts par la même mécanique :
- reset TCP CLOB (v2, 18:11) → reconnexion auto ;
- micro-silences RTDS ~6 s (5 épisodes) → réabonnement, zéro perte ;
- connexion « à moitié morte » (04/07 22:20, 40 min) → **cause du correctif**
  de reconnexion forcée, revalidé par ce chaos test.

## 4. Intégrité des données après incident

- Le journal NDJSON v2 s'écrit en append ; une interruption ne corrompt
  jamais les lignes déjà écrites (une ligne = une trame autonome).
- Redémarrage du bot : réouverture en append du segment horaire courant,
  aucun écrasement (validé sur les 8 (re)démarrages de la journée).
- Le strike d'une fenêtre dont T0 est tombé pendant une panne reste à
  confidence dégradée → fenêtre exclue du trading, signalée dans les logs.

## 5. Limites connues (à garder en tête pour le dry run long)

1. Reconnexion RTDS post-panne : jusqu'à ~60 s si la panne a duré (backoff
   15 s max + timeouts) — sans danger (fenêtre suivante saine), mais
   optimisable.
2. Espace disque : prévoir ~35 Go/jour de marge ou activer l'archivage
   compressé périodique (script campagne : compression + suppression du
   brut à chaque tranche).
3. Horloge système : le strike dépend des timestamps SOURCE (Chainlink),
   pas de l'horloge locale — mais les mesures de latence, elles, supposent
   une horloge correcte : activer chrony (voir doc Fedora).
4. Le mode `--maker` reste expérimental et perdant au backtest : ne pas
   l'activer pour le dry run long.
