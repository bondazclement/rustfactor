# Installation & dry run de A à Z — PC AMD sous Fedora 43

Guide complet pour installer, vérifier, lancer et superviser le dry run
(paper trading) sur votre machine. Aucune clé, aucun fonds, aucun ordre
réel : le bot est compilé en mode DryRun — le mode réel exigerait une
recompilation explicite (`--features live`), documentée ailleurs et
volontairement absente d'ici.

---

## 1. Installation en une ligne

Ouvrez un terminal, puis :

```bash
git clone https://github.com/bondazclement/rustfactor.git && cd rustfactor/polymarket-btc5m-bot && ./install.sh
```

> Le dépôt est privé : si `git clone` demande une authentification,
> connectez-vous une fois avec `gh auth login` (paquet `gh` : `sudo dnf
> install gh`) ou configurez votre clé SSH GitHub, puis relancez la ligne.

Si le projet est déjà cloné (mise à jour) :

```bash
cd rustfactor && git pull && cd polymarket-btc5m-bot && ./install.sh
```

L'installateur est **idempotent** (relançable sans risque) et fait, dans
l'ordre : dépendances système via dnf (gcc, git, curl, zstd, openssl-devel,
pkgconf, lsof, chrony), Rust stable via rustup si absent, compilation
release, exécution des 72 tests, installation du CLI `pm-ctl` dans
`~/.local/bin`. Il active aussi `chronyd` (horloge synchronisée = mesures
de latence fiables).

Matériel : tout PC récent suffit largement — mesuré en production :
**19 Mo de RAM, ~2 % d'un cœur**. Le seul poste à surveiller est le disque
(§6).

## 2. Vérifications avant lancement (2 minutes)

```bash
pm-ctl verifier    # 72 tests locaux + connectivité Polymarket
pm-ctl sante       # détail connectivité : Gamma, CLOB, RTDS
```

Attendu : `72 tests réussis`, puis trois lignes « joignable » (HTTP
301/200/426 — le 426 sur ws-live-data est normal, c'est un endpoint
WebSocket). Si un host est injoignable : vérifiez pare-feu/VPN/proxy ;
le bot lit `HTTPS_PROXY` s'il est défini (tunnel CONNECT intégré).

## 3. Configurer (optionnel — tout a un défaut calibré)

```bash
pm-ctl config           # voir la configuration effective
pm-ctl config init      # créer config.toml (chaque clé documentée en français)
pm-ctl config editer    # l'éditer
```

**Tous** les paramètres sont exposés : capital (`bankroll`, `max_notional`,
`kelly_fraction`), seuils du taker (prix max, z, edge…), maker complet,
modèle probabiliste (plafond de drift, plancher de vol), reconstruction de
volatilité, moteur (cadences, watchdog). Référence exhaustive :
`docs/CONFIGURATION.md`. Surcharges ponctuelles sans fichier :
`pm-ctl direct --bankroll 5000 --max-notional 100`.
Le même `config.toml` est lu par le backtest — validez vos réglages sur
archives avant le live : `pm-ctl backtest`.

## 4. Lancer le dry run

### Mode recommandé : boucle de campagnes (arrière-plan, auto-archivé)

```bash
pm-ctl demarrer --max-entry 0.75 --cible 20
```

- tranches de ~50 min enchaînées automatiquement,
- à chaque fin de tranche : bilan (entrées, PnL, confirmations) ajouté à
  `data_samples_campaign/campaign_summary.log`, journal brut compressé
  (zstd) dans `data_v2/archives/`, version légère dans
  `data_samples_campaign/`,
- arrêt automatique à la cible d'entrées (ou au plafond de tranches),
- le compteur survit aux redémarrages (`data_v2/campaign_state.txt`).

Options : `--max-entry X` (plafond de prix d'entrée taker, défaut 0.75),
`--cible N` (défaut 20), `--tranches N` (défaut 30), `--duree S`
(défaut 3000 s).

Pour un dry run de plusieurs jours : `--cible 200 --tranches 500`.

### Mode direct (premier plan, pour observer)

```bash
pm-ctl direct                      # config par défaut calibrée
pm-ctl direct --max-entry 0.85     # variante prix d'entrée
pm-ctl direct --maker              # ré-active le maker (déconseillé, perdant)
```

Ctrl-C arrête proprement (journaux flushés).

### Arrêter / relancer

```bash
pm-ctl arreter                     # stoppe boucle + bot proprement
pm-ctl demarrer                    # repart du cumul sauvegardé
```

## 5. Superviser

```bash
pm-ctl statut          # état complet : processus, fenêtre, strike, flux,
                       #   résolutions ✓/✗, entrées, PnL, disque
pm-ctl rapport 30      # 30 derniers règlements + toutes les entrées taker
pm-ctl suivre          # logs en direct (Ctrl-C : quitte SANS arrêter le bot)
```

Signaux à connaître dans `statut` :
- `Résolutions : N ✓ / 0 ✗` → chaîne de données saine. **Un ✗** signifie
  que l'issue estimée a contredit la résolution officielle : presque
  toujours la trace d'une coupure réseau passée (fenêtre au strike
  dégradé, automatiquement non tradée). Plusieurs ✗ d'affilée = vérifier
  `pm-ctl sante`.
- `strike gelé … confidence=1.000, gap=0 ms` → parfait (tick Chainlink
  exactement à T0). `confidence=0.000` → fenêtre exclue du trading
  (démarrage en cours de fenêtre ou trou de données : comportement voulu).

## 6. Espace disque (le seul vrai point d'attention)

| Poste | Volume |
| --- | --- |
| Journal brut en cours d'écriture | ~1,3 Go/heure |
| Après compression auto de fin de tranche | ~110 Mo/heure |
| Dry run de 7 jours (boucle, compression auto) | ~18 Go |

La boucle compresse et supprime le brut à chaque tranche. En mode
`direct`, pensez à `pm-ctl compresser` de temps en temps.

## 7. Lancement automatique au démarrage (optionnel, systemd)

```bash
mkdir -p ~/.config/systemd/user
cat > ~/.config/systemd/user/pm-dryrun.service <<'EOF'
[Unit]
Description=Bot Polymarket btc-updown-5m (dry run)
After=network-online.target

[Service]
Type=forking
ExecStart=%h/.local/bin/pm-ctl demarrer --cible 200 --tranches 500
ExecStop=%h/.local/bin/pm-ctl arreter
RemainAfterExit=yes

[Install]
WantedBy=default.target
EOF
systemctl --user daemon-reload
systemctl --user enable --now pm-dryrun
loginctl enable-linger "$USER"   # survit à la fermeture de session
```

## 8. Analyse & calibration sur vos données

```bash
pm-ctl backtest                          # rejoue toutes les archives locales
pm-ctl backtest --max-entry 0.85         # comparer les plafonds de prix
pm-ctl backtest --taker-grid             # grille complète taker
pm-ctl backtest --no-drift               # sensibilité au drift
pm-ctl cadence data_v2/archives/<fichier décompressé>.ndjson
```

## 9. Dépannage

| Symptôme | Cause probable | Remède |
| --- | --- | --- |
| `sante` : host injoignable | pare-feu/VPN/DNS | tester `curl -v https://clob.polymarket.com/` ; si proxy d'entreprise, exporter `HTTPS_PROXY` |
| ✗ répétés dans `statut` | coupure réseau prolongée | `pm-ctl sante`, puis laisser le bot se resynchroniser (2 fenêtres) |
| `strike … confidence=0.000` persistant | flux RTDS muet | vérifier `pm-ctl suivre` : lignes « reconnexion forcée » présentes ? sinon redémarrer (`arreter`/`demarrer`) |
| disque plein | brut non compressé | `pm-ctl compresser` ; augmenter la fréquence des tranches |
| build qui échoue sur openssl | dépendance manquante | `sudo dnf install openssl-devel pkgconf-pkg-config` |

## 10. Garanties de sûreté du dry run

- **Aucun ordre réel possible** : la passerelle live n'est pas compilée.
- Coupure Polymarket totale testée en conditions réelles (audit du
  05/07/2026, `docs/AUDIT_ROBUSTESSE.md`) : détection < 6 s, zéro décision
  pendant la panne, reconnexion autonome, fenêtres suspectes exclues.
- Toutes les trames réseau sont archivées verbatim : chaque décision est
  rejouable et auditable après coup (`pm-ctl backtest`).
