# Plan d'architecture — chantiers parallèles au dry run du 06/07 21h

## Chantier A — `pm-dash` : interface de pilotage web locale

**Choix structurants**
- **Web local (`http://127.0.0.1:7777`)** plutôt que fenêtres natives :
  zéro dépendance desktop, consultable en SSH, sobre par construction.
- **Lecture seule vis-à-vis du bot** : pm-dash lit les artefacts que le
  bot produit déjà (journaux NDJSON, run.log, calibration.json,
  campaign_summary.log). Aucune connexion au processus, aucun impact sur
  le chemin de trading — l'interface peut crasher, le bot ne le saura
  jamais. (Évolution possible plus tard : socket de télémétrie.)
- **Écriture UNIQUEMENT sur `config.toml`**, avec validation par le même
  parseur que le bot (`BotConfig`) + sauvegarde de l'ancienne version.
  Sémantique affichée honnêtement : appliqué au prochain (re)démarrage.
- **Auto-contenu** : HTML/CSS/JS embarqués dans le binaire, graphiques
  SVG maison — fonctionne hors ligne, aucune CDN (cohérent avec la CSP
  du projet : tout local).
- Crate `crates/pm-dash` (axum), réutilise `pm-core` (vol EWMA, strike,
  parse) et `pm-strategy` (config, calib) — les chiffres affichés sont
  calculés par le MÊME code que le bot, pas une réimplémentation.

**Pages/endpoints**
| Route | Contenu |
|---|---|
| `GET /` | page unique : vue d'ensemble + graphiques + menus |
| `GET /api/etat` | fenêtre, strike+confidence, spot, écart $, τ, σ, z, p brute/calibrée, EV, flux, position frontière, PnL, entrées |
| `GET /api/bougies` | OHLC 10 s du prix de résolution + strike + frontières de fenêtres |
| `GET /api/marche` | séries best bid/ask Up et Down (reconstruction Polymarket) |
| `GET /api/vol` | σ EWMA (par √s) dans le temps |
| `GET /api/calibration` | table bacs $×τ : p̂, effectifs (heatmap) |
| `GET/POST /api/config` | lecture de la config EFFECTIVE ; écriture validée de config.toml |

**Contraintes de build pendant le dry run** : compilation dans
`CARGO_TARGET_DIR` séparé — le binaire du run n'est jamais touché.

## Chantier B — passation vers Claude Code / Opus 4.8

**Livrables**
1. `CLAUDE.md` (racine dépôt + racine bot) : carte du projet, invariants
   NON NÉGOCIABLES, état exact, commandes, pièges connus ;
2. `docs/VISION.md` : trajectoire, questions de recherche ouvertes avec
   état des preuves, et le principe méthodologique central du projet :
   **les données priment sur l'intention du prompt** (exemples vécus) ;
3. `docs/DECISIONS.md` : journal chronologique de chaque décision de
   modèle AVEC la mesure qui l'a motivée — y compris les idées réfutées
   (pour ne pas les re-tester naïvement) ;
4. Index `docs/README.md` réorganisé par usage (démarrer / comprendre /
   décider / historique).

**Méthode** : les caractéristiques d'Opus 4.8 et les bonnes pratiques
CLAUDE.md sont sourcées depuis la documentation Anthropic (agent de
recherche dédié), pas de mémoire — voir résultats en fin de document.

## Ordonnancement (pendant le run d'1 h)
supervision (fond) → plan (ce fichier) → recherche doc (fond) →
pm-dash (build séparé + review) → docs passation → reviews croisées →
rapport final avec le bilan du dry run.
