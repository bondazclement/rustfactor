# rustfactor — bot de trading Polymarket btc-updown-5m

Tout le projet vit dans `polymarket-btc5m-bot/` (workspace Rust, 7 crates).
Lis `polymarket-btc5m-bot/CLAUDE.md` avant toute modification.

## Règles absolues (ne jamais enfreindre)

- **AUCUN dry run ni processus de trading ne se lance sans confirmation
  explicite de l'utilisateur dans la conversation en cours.**
- Le mode réel exige un triple opt-in : `--features live` + `--live` +
  `PM_LIVE_ARME=oui`. Ne jamais affaiblir un garde-fou de `risk.rs`.
- Les archives NDJSON sont la vérité : jamais de troncature/réécriture.
- Chaque affirmation sur le marché doit être MESURÉE sur les données
  avant d'être codée (voir docs/DECISIONS.md : la moitié des intuitions
  initiales ont été réfutées par la mesure).
