# Configurer les credentials Polymarket pour le bot — pas à pas

Fiche pour un compte Polymarket créé avec **connexion Google** (ou e-mail
Magic Link) — votre cas. Pour les autres types de comptes, voir le tableau
du §2.

---

## 0. Sécurité — à lire avant tout

- La clé privée que vous allez exporter **contrôle tous les fonds du
  compte**. Elle ne se partage jamais, ne se stocke jamais en clair, ne se
  colle jamais dans un chat, un mail ou un fichier du dépôt git.
- Le script du bot (`scripts/tester-ordres.sh`) la demande en **saisie
  masquée** et ne la garde qu'en mémoire le temps du test — rien n'est
  écrit sur disque. Vérifiez-le vous-même : le script fait 50 lignes.
- Pour la phase de test, gardez sur le compte **un montant minime**
  (~20-30 $) : c'est votre vrai plafond de risque, quelle que soit la
  qualité des garde-fous logiciels.
- Ne tapez jamais la clé dans un terminal dont l'historique est actif
  (`export POLYMARKET_PRIVATE_KEY=…` resterait dans `~/.bash_history` —
  le script évite ce piège en la demandant interactivement).

## 1. Ce dont le bot a besoin (3 informations)

| Variable | C'est quoi | Où la trouver |
|---|---|---|
| `POLYMARKET_PRIVATE_KEY` | La clé privée du **signer** (le wallet intégré créé par votre connexion Google) | §3 |
| `POLYMARKET_FUNDER` | L'adresse du **portefeuille Polymarket** qui détient vos fonds (proxy) | §4 |
| `POLYMARKET_SIG_TYPE` | Le type de signature qui relie les deux | `1` pour un compte Google/e-mail (§2) |

## 2. Identifier son type de compte

| Comment vous vous connectez à polymarket.com | Type | `POLYMARKET_SIG_TYPE` |
|---|---|---|
| **Google / e-mail (Magic)** ← votre cas | Proxy Polymarket | **1** |
| MetaMask ou autre wallet navigateur (compte ancien) | Gnosis Safe | 2 |
| Clé privée gérée par vous, fonds sur l'adresse elle-même | EOA | 0 |
| « Deposit wallet » créé via l'API (nouveaux intégrateurs) | Poly1271 | 3 |

Référence : docs.polymarket.com/api-reference/authentication
(« POLY_PROXY (1) : commonly used by users who logged in via Magic Link
email/Google »).

## 3. Exporter la clé privée (signer)

1. Connectez-vous sur **polymarket.com** avec Google, comme d'habitude.
2. Cliquez sur votre avatar/profil (en haut à droite) → **Settings**.
3. Cherchez la section **« Export Private Key »** (parfois sous
   *Wallet* ou *Advanced*). C'est la fonction d'export du wallet intégré
   Magic — elle vous fait confirmer votre identité puis **révèle une clé
   `0x…` de 66 caractères**.
4. Copiez-la temporairement (gestionnaire de mots de passe, PAS un
   fichier texte). C'est elle que le script demandera en saisie masquée.

> Si l'option n'apparaît pas : Polymarket fait passer l'export par
> reveal.magic.link (le fournisseur du wallet intégré) — suivez le lien
> proposé dans Settings, authentifiez-vous avec le MÊME compte Google, la
> clé s'affiche.

## 4. Récupérer l'adresse funder (votre portefeuille Polymarket)

1. Toujours sur polymarket.com, ouvrez votre profil (ou le bouton
   **Deposit**).
2. Copiez **l'adresse `0x…` affichée comme votre adresse Polymarket**
   (celle vers laquelle vous déposez des fonds, visible aussi sous votre
   nom de profil → « copy address »).
3. C'est le `POLYMARKET_FUNDER`. ⚠️ Ce n'est PAS l'adresse dérivée de la
   clé privée du §3 : la clé signe, le proxy détient les fonds — les deux
   adresses sont différentes, c'est normal (c'est ce que le type 1
   déclare).

## 5. Approvisionnement (déjà fait si vous avez déjà tradé)

- Le compte doit contenir des fonds (pUSD/USDC) : **~20-30 $ suffisent**
  pour toute la phase de test. Un dépôt par carte/crypto via le bouton
  Deposit de l'interface fait l'affaire.
- Pas besoin de POL/MATIC pour le gas avec un compte type 1 (transactions
  relayées par Polymarket).
- Si vous avez déjà passé UN trade via l'interface web, les approbations
  de contrats sont déjà en place. Sinon, faites un micro-trade quelconque
  dans l'UI une fois (1 $), c'est le plus simple pour les créer.

## 6. Lancer le test des ordres

```bash
cd "~/Documents/Projet code/rustfactor/polymarket-btc5m-bot"
./scripts/tester-ordres.sh
```

Le script vous demande dans l'ordre :
1. **La clé privée** (saisie masquée, rien ne s'affiche — c'est voulu) ;
2. **L'adresse funder** → collez l'adresse du §4 ;
3. **Le type de signature** → tapez `1`.

Puis il déroule automatiquement : découverte de la fenêtre btc-updown-5m
active → authentification → lecture du solde → pose d'un ordre GTC de
5 parts à 0,01 $ (**5 centimes immobilisés, inexécutable** sur un carnet
coté ~0,50) → vérification qu'il est visible → annulation → vérification
que le carnet est propre → rapport des latences. Durée : ~15 secondes.

**Résultat attendu** : 7 × PASS et `TOUT PASSE — chemin d'ordres réel
opérationnel`, avec vos latences réelles (auth, POST, annulation).

## 7. Dépannage

| Erreur | Cause probable | Remède |
|---|---|---|
| `Invalid Signature` / `L2 AUTH NOT AVAILABLE` | type de signature ou funder qui ne correspondent pas à la clé | vérifiez : clé du §3 + adresse du §4 + type `1` |
| `insufficient balance` au test de solde | compte vide | déposez ~20 $ via l'UI |
| `not enough allowance` | jamais tradé via l'UI | faites 1 micro-trade dans l'interface web, puis relancez |
| Erreur Cloudflare / geoblock | l'API d'ordres refuse les régions restreintes ; le test emprunte votre chemin réseau actuel | c'est la contrainte réglementaire déjà discutée — elle s'applique aux ordres réels comme au site |
| `clé privée invalide` | espace/retour à la ligne collé avec la clé | recollez sans espaces (le script fait déjà un `trim`) |

## 8. Ce que ce test ne fait PAS

Aucun trade exécutable : l'ordre est posé à 0,01 $ sur un marché coté
autour de 0,50 et annulé aussitôt. Exposition maximale théorique :
**0,05 $ pendant ~2 secondes**. Le passage à de vrais micro-trades
(≤ 20 $) est une étape séparée, déclenchée par vous, avec le RiskGate
configuré en conséquence.
