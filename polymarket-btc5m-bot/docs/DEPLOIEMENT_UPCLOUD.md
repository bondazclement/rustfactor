# Déploiement sur UpCloud (Ubuntu Server 26.04, datacenter Stockholm)

Guide pas à pas pour faire tourner le bot 24/7 sur un serveur UpCloud, en
dry run (paper) puis, quand vous le décidez, en micro-test réel. Basé sur
la documentation officielle UpCloud (liens en bas).

> **Pourquoi un VPS hors de France** : Polymarket est bloqué par certains
> FAI français (inspection SNI). Un serveur dans un datacenter suédois
> résout la connectivité de collecte. La question réglementaire de
> l'exécution réelle depuis votre juridiction vous appartient — ce guide
> ne traite que de l'aspect technique.

---

## 1. Créer le serveur (panneau de contrôle)

1. Connectez-vous à **hub.upcloud.com** → bouton **« Deploy server »**.
2. **Location** : choisissez **Stockholm** (Suède). Le code de zone est
   `se-sto1` (visible dans le sélecteur ; en CLI c'est cette valeur).
3. **Plan** : onglet **Premium** (haute fréquence CPU, stockage MaxIOPS).
   Le bot est frugal (~20 Mo RAM, ~2 % d'un cœur) ; **Premium 1 CPU / 1 Go
   RAM suffit largement**. Prenez 2 CPD/2 Go si vous voulez compiler plus
   vite et faire tourner des backtests en parallèle.
4. **Storage** : MaxIOPS. **80 Go** confortables (un dry run de 7 jours =
   ~18 Go de journaux bruts, compressés automatiquement ; voir §6).
5. **Automated backups** (optionnel) : plan *Day* si vous voulez une
   sauvegarde quotidienne.
6. **Operating system** : **Ubuntu Server 26.04 LTS**.
7. **SSH keys** : ajoutez votre clé publique (`~/.ssh/id_ed25519.pub`).
   C'est la méthode de connexion requise sous Linux — n'activez pas le mot
   de passe.
8. **Hostname** : ex. `pm-bot.mondomaine` (ou laissez le défaut).
9. **Deploy**.

### Équivalent en ligne de commande (`upctl`)

Si vous préférez le CLI (`brew install upcloud/tap/upcloud-cli` ou binaire
officiel) :

```bash
upctl server create \
  --zone se-sto1 \
  --plan 2xCPU-2GB \
  --os "Ubuntu Server 26.04 LTS (Noble Numbat)" \
  --os-storage-size 80 \
  --ssh-keys ~/.ssh/id_ed25519.pub \
  --create-password false \
  --hostname pm-bot --title "pm-bot"
```

> Le libellé exact de l'OS et le nom du plan peuvent varier ;
> `upctl server plans` et `upctl zone list` donnent les valeurs à jour.

## 2. Première connexion & socle système

```bash
ssh ubuntu@<IP-DU-SERVEUR>     # l'utilisateur par défaut Ubuntu sur UpCloud
sudo apt-get update && sudo apt-get -y upgrade
sudo timedatectl set-timezone UTC          # horodatage cohérent des logs
```

Créez de préférence un utilisateur non-root dédié (le bot n'a jamais
besoin de root) :

```bash
sudo adduser --disabled-password --gecos "" pm
sudo cp -r ~/.ssh /home/pm/ && sudo chown -R pm:pm /home/pm/.ssh
sudo -iu pm
```

## 3. Installer le bot

```bash
git clone <URL-DE-VOTRE-DEPOT>.git polymarket-btc5m-bot
cd polymarket-btc5m-bot
./install.sh --auto          # installation directe, sans TTY (idéale en SSH)
# ou ./install.sh pour la console interactive multilingue
export PATH="$HOME/.local/bin:$PATH"   # ajoutez-le à ~/.bashrc
```

`--auto` installe les dépendances (apt), Rust, compile, lance les tests et
installe `pm-ctl`. Comptez ~5-10 min sur un plan 2 CPU (la compilation Rust
est le poste dominant).

## 4. Vérifier la connectivité (le point clé du VPS)

```bash
pm-ctl sante
```

Attendu : trois lignes « joignable » (Gamma, CLOB, ws-live-data). Depuis
un datacenter suédois, aucun blocage SNI — si un host échoue, vérifiez le
pare-feu UpCloud (§7).

## 5. Lancer le dry run 24/7 (service systemd)

Pour que le bot survive aux déconnexions SSH et redémarre au boot :

```bash
mkdir -p ~/.config/systemd/user
cat > ~/.config/systemd/user/pm-dryrun.service <<'EOF'
[Unit]
Description=Bot Polymarket btc-updown-5m (dry run)
After=network-online.target

[Service]
Type=forking
WorkingDirectory=%h/polymarket-btc5m-bot
ExecStart=%h/.local/bin/pm-ctl demarrer --cible 500 --tranches 1000
ExecStop=%h/.local/bin/pm-ctl arreter
RemainAfterExit=yes

[Install]
WantedBy=default.target
EOF
systemctl --user daemon-reload
systemctl --user enable --now pm-dryrun
loginctl enable-linger "$USER"     # le service tourne même sans session SSH
```

Supervision à distance :

```bash
pm-ctl statut        # état complet
pm-ctl suivre        # logs en direct
journalctl --user -u pm-dryrun -f
```

## 6. Espace disque

| Poste | Volume |
|---|---|
| Journal brut en cours | ~1,3 Go/h |
| Après compression auto (fin de tranche) | ~110 Mo/h |
| Dry run de 7 jours (compression auto) | ~18 Go |

La boucle de campagne compresse (zstd) et purge le brut à chaque tranche.
80 Go de disque couvrent largement plusieurs semaines. Surveillez avec
`pm-ctl statut` (ligne disque).

## 7. Pare-feu UpCloud

Le bot n'a besoin d'AUCUN port entrant (il n'ouvre que des connexions
sortantes vers Polymarket). Configuration recommandée dans le panneau
**Firewall** du serveur :

- **Entrant** : autoriser SSH (port 22) uniquement depuis votre IP ;
  refuser le reste.
- **Sortant** : tout autoriser (HTTPS/WSS vers Polymarket et Binance).
- Le **dashboard web** (`pm-dash`, port 7777) écoute sur `127.0.0.1`
  uniquement : ne l'exposez pas. Pour le consulter depuis votre machine,
  passez par un **tunnel SSH** :
  ```bash
  ssh -L 7777:localhost:7777 pm@<IP-DU-SERVEUR>
  # puis, côté serveur : ~/polymarket-btc5m-bot/target/release/pm-dash ~/polymarket-btc5m-bot
  # et ouvrez http://localhost:7777 dans VOTRE navigateur
  ```

## 8. Passer au micro-test réel (quand vous le décidez)

Rien d'automatique : le mode réel exige la recompilation `--features live`
et vos identifiants. Le script guide tout :

```bash
cd ~/polymarket-btc5m-bot
./scripts/micro-test.sh lancer --duree 8
```

Il demande la clé privée (masquée, jamais stockée), le funder et le type
de signature (voir [`CREDENTIALS.md`](CREDENTIALS.md)), plafonne à 5 $/ordre
et 20 $ de perte max. Réclamation des gains : manuelle sur polymarket.com
pour l'instant (voir [`VISION.md`](VISION.md), chantier auto-redeem).

## 9. Sauvegarde de la table de calibration

Le seul état précieux qui s'accumule est `data_v2/calibration.json` (la
mémoire apprise du bot). Sauvegardez-le régulièrement hors du serveur :

```bash
scp pm@<IP-DU-SERVEUR>:~/polymarket-btc5m-bot/data_v2/calibration.json ./calibration-$(date +%F).json
```

---

## Sources UpCloud

- Déploiement d'un serveur : https://upcloud.com/docs/guides/deploy-server/
- Guide de démarrage : https://upcloud.com/docs/guides/quick-start-guide/
- CLI `upctl` : https://upcloud.com/docs/guides/upcloud-command-line-interface/
- Référence `upctl server create` :
  https://upcloudltd.github.io/upcloud-cli/commands_reference/upctl_server/create/
