# polymarket-btc5m-bot — carte du projet pour Claude Code

Bot de trading (paper + réel) sur les marchés Polymarket « Bitcoin Up or
Down » de 5 minutes. Rust, 7 crates, ~92 tests. Tout est en français
(logs, docs, commits).

## Règles absolues

- **Ne lancer AUCUN dry run / bot / campagne sans confirmation explicite
  de l'utilisateur.** (Exigence utilisateur du 06/07/2026.)
- Mode réel = triple opt-in (`--features live` + `--live` +
  `PM_LIVE_ARME=oui`) ; ne jamais affaiblir `pm-execution/src/risk.rs`.
- Une seule source de vérité marché : le flux de résolution Chainlink
  (RTDS). Archives NDJSON verbatim, jamais modifiées.
- Toute idée de stratégie se VALIDE sur les archives avant d'être codée
  (`docs/DECISIONS.md` liste les intuitions réfutées — ne pas les
  re-tester naïvement, ne pas les réintroduire).

## Commandes

```bash
cargo build --release && cargo test --workspace   # build + ~92 tests
./scripts/pm-ctl statut|rapport|sante|config      # supervision (FR)
./scripts/pm-ctl demarrer|arreter                 # campagne (CONFIRMATION UTILISATEUR D'ABORD)
CARGO_TARGET_DIR=/tmp/dash cargo run -p pm-dash   # interface localhost:7777
./scripts/tester-ordres.sh                        # batterie chemin réel (credentials)
./target/release/pm-backtest --journal <f.ndjson> --no-maker  # rejouer
bash analysis/backtest_corpus.sh <label>          # corpus complet, mémoire bornée
python3 analysis/extract.py 'data_v2/archives/*.zst'  # extraction compacte
```

## Architecture (crates/)

| Crate | Rôle |
|---|---|
| pm-core | types, parsing, carnet L2, strike (LastAtOrBefore), vol EWMA, math (Student-t) |
| pm-acquisition | WS RTDS (oracle+spot), CLOB, Gamma, Binance direct, recorder verbatim, watchdog |
| pm-strategy | modèle proba + **table de calibration auto-apprise** (calib.rs, bacs $×τ), taker « frontière », maker (OFF), config TOML |
| pm-execution | DryRunGateway / LiveGateway (SDK officiel) / **RiskGate** / pm-live-test |
| pm-bot | orchestrateur événementiel (décision à chaque tick d'oracle) |
| pm-replay | backtest = même moteur que le live, walk-forward + Brier |
| pm-dash | interface web locale, LECTURE SEULE des artefacts |

## État au 07/07/2026 (voir docs/VISION.md pour la suite)

- Modèle v4 « la frontière » : entrée ssi écart ≥ 70 $ ET τ ≤ 120 s ET
  prix ≤ 0,98 ET EV calibrée ≥ 1 pt. Seule règle prouvée à 90 % (34/34,
  +521 $/13 h simulées). Premier corpus walk-forward positif (+83 $).
- Chemin réel VALIDÉ par l'utilisateur (batterie 7/7, ordre réel posé et
  annulé, POST 921 ms). Compte : type signature 1 (Google/Magic).
- Prochaines étapes convenues : micro-test réel ≤ 20 $ (sur GO
  utilisateur uniquement), étude lead-lag Binance direct (données en
  capture), re-design maker (rebates 20 %, frais 0).

## Pièges connus (vécus)

- Backtester UNIQUEMENT sur journaux complets — les archives `_light`
  n'ont ~6 % du carnet → fills fantômes (+13 900 $ fictifs un jour).
- `pm-backtest` charge tout en RAM : corpus par morceaux via
  `analysis/backtest_corpus.sh` (OOM vécu à 9,4 Go).
- Les `raw` des journaux sont du JSON échappé : `grep '"btc/usd"'` ne
  matche PAS (chercher `btc/usd` sans guillemets).
- `pkill -f` : le motif peut matcher votre propre ligne de commande —
  utiliser `pkill -x`.
- Le silence des flux se mesure sur les VRAIS ticks (les PONG masquaient
  une connexion à moitié morte : 45 min de strike figé le 06/07).
- L'espace dans le chemin du dépôt (« Projet code ») casse les scripts
  non quotés.
- Chemin table de calibration : env `PM_CALIB_PATH` (la campagne le fixe
  à data_v2/calibration.json — sinon chaque tranche repartirait à vide).

## Style

Code/commentaires : idiome du dépôt, français, commentaires = contraintes
non évidentes uniquement. Commits : titre impératif + mesures chiffrées
qui justifient le changement.
