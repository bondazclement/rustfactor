# MVP trading réel — architecture, garanties, runbook

État : **implémenté et compilé** (06/07/2026), non encore branché sur un
wallet. Ce document est le mode d'emploi exigeant pour les premiers
micro-trades réels (5-10 $), et la liste honnête de ce qui reste.

## Architecture de l'exécution réelle

```
stratégie (taker EV) ──► Passerelle (enum)
                           ├─ Paper : RiskGate<DryRunGateway>   (défaut)
                           └─ Réelle: RiskGate<LiveGateway>     (--features live + --live)
                                        │
                                        ▼
                        SDK officiel polymarket_client_sdk_v2 0.6
                        (auth L1→L2, tick/neg_risk/frais auto,
                         signature EIP-712 locale, POST CLOB)
```

- **`RiskGate`** (crates/pm-execution/src/risk.rs) — la dernière ligne de
  défense, TOUJOURS active (paper comme réel) et testée unitairement :
  - armement double opt-in : compilation `--features live` ET `--live` ET
    `PM_LIVE_ARME=oui` — trois verrous vérifiés au comportement ;
  - notional max/ordre 10 $, 20 ordres max/session, perte max session 30 $
    → **kill-switch définitif** ;
  - kill-switch automatique sur toute **contradiction de résolution ✗**
    (chaîne de données suspecte = plus aucun ordre, les annulations restent
    permises) ;
  - tout refus est journalisé avec sa raison.
- **`LiveGateway`** (pm-execution, feature `live`) — SDK officiel :
  authentification unique au démarrage, **vérification du solde** (refus si
  < 1 $), ordres limit FAK/FOK/GTC, annulation par marché. La réponse du
  POST contient les quantités réellement exécutées (making/taking amounts,
  journalisées) : pour un taker FAK, c'est la confirmation de fill.
- **Frais réels** : le SDK récupère le fee rate du marché et le modèle les
  intègre dans l'EV (0,07 × p(1−p) crypto) — cohérence paper/réel.

## Runbook : premiers micro-trades (quand VOUS déciderez)

1. Wallet dédié au test, approvisionné de 50 $ max (pUSD Polygon + POL si
   EOA). Jamais un wallet principal.
2. Approbations one-time des contrats (voir docs Polymarket
   `/market-makers/getting-started`) — non automatisé ici, volontairement.
3. ```bash
   cargo build --release --features live
   export POLYMARKET_PRIVATE_KEY=0x…       # clé du wallet de TEST
   export POLYMARKET_FUNDER=0x…            # si type ≠ EOA
   export POLYMARKET_SIG_TYPE=0            # 0 EOA / 1 Proxy / 2 Safe / 3 Poly1271
   export PM_LIVE_ARME=oui
   ./target/release/pm-bot --live --max-notional 10
   ```
4. Suivi : `pm-ctl statut` + interface Polymarket (positions visibles).
   Chaque ordre réel est journalisé avec son order_id et son statut.
5. Arrêt : Ctrl-C, ou automatique (perte 30 $ / 20 ordres / ✗).

## Ce qui manque encore (liste exigeante)

| Manque | Impact | Priorité |
|---|---|---|
| **Un edge démontré** | le moteur honnête ne tradera presque jamais aux prix actuels du carnet — c'est correct, mais un MVP sans edge valide l'EXÉCUTION, pas la rentabilité | — (voir ETUDE_MODELE.md §5.3 : lead-lag spot en cours de capture) |
| Canal WSS `user` (fills temps réel) | le POST FAK renvoie déjà le fill ; le WSS devient nécessaire pour des ordres GTC (maker) | P2 |
| Réconciliation automatique paper↔réel | comparaison manuelle via UI/logs au début (volumes minuscules) | P2 |
| Approbations de contrats scriptées | one-time, faisable via l'UI Polymarket au premier dépôt | P3 |
| Redémarrage avec positions ouvertes réelles | le bot ne relit pas ses positions au boot (fenêtres de 5 min : exposition ≤ 5 min) | P2 |
| Rate limits/heartbeats CLOB | volumes de test négligeables | P3 |

## Verdict de préparation

- **Exécution** : prête à tester avec des micro-montants (garde-fous
  vérifiés, frais exacts, fills confirmés par la réponse d'ordre).
- **Rentabilité** : NON démontrée — l'étude montre l'absence d'edge taker
  aux prix actuels (docs/ETUDE_MODELE.md). Le passage à l'argent réel ne
  doit servir qu'à valider la mécanique d'exécution, pas à « gagner ».
  Le premier euro d'edge viendra du chantier lead-lag spot/oracle ou d'un
  maker repensé — pas avant validation sur données.
