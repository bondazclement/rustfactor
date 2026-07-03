# Rustector_btc_5mn

Programme Rust tout-en-un pour `btc-updown-5m` :

- collecte RTDS Chainlink `btc/usd` + CLOB (orderbook/trades),
- affichage TUI live (prix, bougie, carnets, latence),
- enregistrement NDJSON en parallèle avec état d'écriture visible en direct.

## Installation (une ligne)

```bash
bash install.sh
```

## Lancement

```bash
./target/release/Rustector_btc_5mn
```

Options :

```bash
./target/release/Rustector_btc_5mn --out ../data_low_latency
./target/release/Rustector_btc_5mn --duration-sec 320
./target/release/Rustector_btc_5mn --no-record
```

## Données enregistrées

Par fenêtre active :

```text
<out>/window_<epoch>/raw.ndjson
```

Le footer affiche en direct :

- état REC (`ON`, `idle`, `OFF`),
- lignes/bytes écrits,
- chemin de fichier courant.
