#!/usr/bin/env python3
"""Audit de latence de toute la chaîne (phase 2).
Sources : journaux du soir (oracle + relais spot + Binance direct)."""
import glob
import io
import json
import subprocess
import numpy as np

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

lat_oracle, lat_relay, lat_binance, lat_clob = [], [], [], []
gap_oracle = []
last_oracle = None
for path in sorted(glob.glob("data_v2/camp_20260706T1*/journal_*.ndjson")):
    for fr in frames(path):
        s, raw, rm = fr.get("stream"), fr.get("raw", ""), fr.get("recv_ms", 0)
        if s == "rtds":
            if '"btc/usd"' in raw:
                try:
                    m = json.loads(raw)
                    ts = int(m["payload"]["timestamp"])
                    lat_oracle.append(rm - ts)
                    if last_oracle: gap_oracle.append(ts - last_oracle)
                    last_oracle = ts
                except Exception: pass
            elif '"btcusdt"' in raw:
                try:
                    m = json.loads(raw)
                    lat_relay.append(rm - int(m["payload"]["timestamp"]))
                except Exception: pass
        elif s == "binance":
            try:
                m = json.loads(raw)
                if "E" in m: lat_binance.append(rm - int(m["E"]))
            except Exception: pass
        elif s == "clob" and '"price_changes"' in raw:
            try:
                m = json.loads(raw)
                t = int(m.get("timestamp", m.get("price_changes", [{}])[0].get("timestamp", 0)))
                if t > 1e12: lat_clob.append(rm - t)
            except Exception: pass

def stats(name, v, unit="ms"):
    if not v:
        print(f"{name:<40} (aucune donnée)")
        return
    v = np.array(v, dtype=float)
    print(f"{name:<40} n={len(v):>7} p50={np.percentile(v,50):>7.0f}{unit} p90={np.percentile(v,90):>7.0f}{unit} p99={np.percentile(v,99):>7.0f}{unit}")

print("── Latences de réception (timestamp source → notre horloge) ──")
stats("Oracle Chainlink (via RTDS)", lat_oracle)
stats("Spot btcusdt (via relais RTDS Polymarket)", lat_relay)
stats("Spot btcusdt (BINANCE DIRECT, event time)", lat_binance)
stats("Carnet CLOB (price_change, ts serveur)", lat_clob)
print("── Cadence oracle (écart entre ticks source) ──")
stats("Intervalle entre ticks oracle", gap_oracle)
