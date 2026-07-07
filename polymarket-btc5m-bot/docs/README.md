# Index de la documentation — par usage

## Démarrer
| Doc | Contenu |
|---|---|
| [INSTALLATION_FEDORA.md](INSTALLATION_FEDORA.md) | installer, vérifier, lancer, superviser (A à Z) |
| [CONFIGURATION.md](CONFIGURATION.md) | référence des ~40 paramètres (tout est configurable) |
| [CREDENTIALS.md](CREDENTIALS.md) | credentials compte Google/Magic pas à pas (type 1) |
| `../CLAUDE.md` | carte du projet + règles absolues + pièges (pour Claude Code) |

## Comprendre l'état du projet
| Doc | Contenu |
|---|---|
| [HISTORIQUE.md](HISTORIQUE.md) | **la lignée : Python → Rustector → v4, ce que chaque version a appris** |
| [VISION.md](VISION.md) | **trajectoire long terme + questions de recherche ouvertes** |
| [DECISIONS.md](DECISIONS.md) | **chaque décision avec sa preuve + idées réfutées** |
| [MODELE_V3.md](MODELE_V3.md) | le modèle (v3 + addendum v4 « frontière ») |
| [ARCHITECTURE.md](ARCHITECTURE.md) | les 7 crates, flux de données |

## Les études (chronologique — les preuves brutes)
| Doc / script | Résultat central |
|---|---|
| [PHASE1_FINDINGS.md](PHASE1_FINDINGS.md) | formats de données, price-to-beat |
| [VALIDATION_LIVE.md](VALIDATION_LIVE.md) | strike 5/5 exact, chaîne de résolution validée |
| [ETUDE_MODELE.md](ETUDE_MODELE.md) | kurtosis 238, marché mieux calibré, edge taker négatif |
| [LIGNE_EFFICIENCE.md](LIGNE_EFFICIENCE.md) | LA frontière prouvée (écart×τ×prix) |
| [AUDIT_VITESSE.md](AUDIT_VITESSE.md) | latences mesurées, moteur événementiel, A/B |
| [AUDIT_ROBUSTESSE.md](AUDIT_ROBUSTESSE.md) | coupure réseau totale en conditions réelles |
| `../analysis/*.py` | scripts reproductibles de toutes les études |

## Déployer
| Doc | Contenu |
|---|---|
| [DEPLOIEMENT_UPCLOUD.md](DEPLOIEMENT_UPCLOUD.md) | VPS UpCloud (Ubuntu 26.04, Stockholm) pas à pas |

## Exécution réelle
| Doc | Contenu |
|---|---|
| [MVP_REEL.md](MVP_REEL.md) | architecture LiveGateway/RiskGate, runbook, manques |
| `../scripts/tester-ordres.sh` | batterie A-Z du chemin d'ordres (validée 7/7 le 06/07) |

## Historique
[ROADMAP.md](ROADMAP.md) · [RAPPORT_NUIT_20260706.md](RAPPORT_NUIT_20260706.md) ·
[PLAN_CHANTIERS_DASH_DOC.md](PLAN_CHANTIERS_DASH_DOC.md) · data_samples/README.md
