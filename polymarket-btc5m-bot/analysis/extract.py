#!/usr/bin/env python3
"""Extracteur compact des journaux NDJSON v2 (bruts ou .zst).

Produit trois CSV dans analysis/out/ :
  ticks.csv    : ts_ms,price,recv_ms          (Chainlink btc/usd, dédupliqué)
  spot.csv     : ts_ms,price,recv_ms          (Binance btcusdt, dédupliqué)
  tops.csv     : recv_ms,asset,bid,ask        (top-of-book, échantillonné 1 Hz/asset)
  windows.csv  : slug,t0_ms,end_ms,token_up,token_down

Streaming pur : jamais plus d'une trame en mémoire.
"""
import glob
import io
import json
import os
import subprocess
import sys

BASE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(BASE, "out")
os.makedirs(OUT, exist_ok=True)


def frames(path):
    if path.endswith(".zst"):
        p = subprocess.Popen(["zstdcat", path], stdout=subprocess.PIPE)
        f = io.TextIOWrapper(p.stdout, errors="replace")
    else:
        f = open(path, errors="replace")
    for line in f:
        try:
            yield json.loads(line)
        except json.JSONDecodeError:
            continue
    f.close()


def main(patterns):
    paths = sorted(set(sum((glob.glob(p) for p in patterns), [])))
    print(f"{len(paths)} journaux", file=sys.stderr)
    seen_tick = set()      # ts_ms Chainlink déjà émis
    seen_spot = set()
    windows = {}           # slug -> ligne
    last_top = {}          # asset -> (sec, bid, ask) dernier émis
    n_frames = 0
    with open(f"{OUT}/ticks.csv", "w") as ft, open(f"{OUT}/tops.csv", "w") as fb, \
         open(f"{OUT}/spot.csv", "w") as fs:
        ft.write("ts_ms,price,recv_ms\n")
        fb.write("recv_ms,asset,bid,ask\n")
        fs.write("ts_ms,price,recv_ms\n")
        for path in paths:
            for fr in frames(path):
                n_frames += 1
                s = fr.get("stream")
                raw = fr.get("raw", "")
                if s == "rtds":
                    est_chainlink = '"btc/usd"' in raw
                    est_spot = '"btcusdt"' in raw
                    if not est_chainlink and not est_spot:
                        continue
                    try:
                        m = json.loads(raw)
                        pl = m["payload"]
                        ts = int(pl["timestamp"])
                        if est_chainlink and ts not in seen_tick:
                            seen_tick.add(ts)
                            ft.write(f"{ts},{pl['value']},{fr['recv_ms']}\n")
                        elif est_spot and ts not in seen_spot:
                            seen_spot.add(ts)
                            fs.write(f"{ts},{pl['value']},{fr['recv_ms']}\n")
                    except (KeyError, ValueError, TypeError):
                        continue
                elif s == "clob":
                    if '"price_changes"' not in raw:
                        continue
                    try:
                        m = json.loads(raw)
                        rm = fr["recv_ms"]
                        sec = rm // 1000
                        for ch in m.get("price_changes", []):
                            a = ch["asset_id"][:12]
                            bid, ask = ch.get("best_bid"), ch.get("best_ask")
                            if not bid or not ask:
                                continue
                            prev = last_top.get(a)
                            if prev and prev[0] == sec and prev[1] == bid and prev[2] == ask:
                                continue
                            last_top[a] = (sec, bid, ask)
                            fb.write(f"{rm},{a},{bid},{ask}\n")
                    except (KeyError, ValueError, TypeError):
                        continue
                elif s == "gamma":
                    try:
                        m = json.loads(raw)
                        windows[m["slug"]] = (
                            m["slug"], m["start_ms"], m["end_ms"],
                            m["token_up"][:12], m["token_down"][:12],
                        )
                    except (KeyError, ValueError, TypeError):
                        continue
    with open(f"{OUT}/windows.csv", "w") as fw:
        fw.write("slug,t0_ms,end_ms,token_up,token_down\n")
        for w in sorted(windows.values(), key=lambda x: x[1]):
            fw.write(",".join(map(str, w)) + "\n")
    print(f"{n_frames} trames | {len(seen_tick)} ticks | {len(seen_spot)} spot | {len(windows)} fenêtres", file=sys.stderr)


if __name__ == "__main__":
    main(sys.argv[1:])
