#!/usr/bin/env python3
"""Étude 3 : gisements d'edge microstructure.
A) Après un SAUT d'oracle (|r_1s| > 4σ), en combien de temps le carnet
   re-price-t-il ? Y a-t-il une fenêtre de latence exploitable ?
B) Violations de parité : ask_up + ask_down < 1 (achat des deux côtés
   = gain sans risque) ; fréquence, profondeur.
"""
import numpy as np
import pandas as pd

OUT = "analysis/out"
ticks = pd.read_csv(f"{OUT}/ticks.csv").sort_values("ts_ms").reset_index(drop=True)
tops = pd.read_csv(f"{OUT}/tops.csv")
win = pd.read_csv(f"{OUT}/windows.csv")
mom = pd.read_csv(f"{OUT}/moments_mkt.csv")

lp = np.log(ticks.price.to_numpy())
ts = ticks.ts_ms.to_numpy()
rc = ticks.recv_ms.to_numpy()
dt = np.diff(ts) / 1000.0
ret = np.diff(lp)
sd = ret[(dt >= 0.5) & (dt <= 1.5)].std()

# latence de réception de l'oracle chez nous
lat = rc - ts
print(f"latence tick oracle→nous: p50={np.percentile(lat,50):.0f}ms p90={np.percentile(lat,90):.0f}ms")

# ── A. Sauts et réaction du carnet ──────────────────────────────────────
amap = {}
for w in win.itertuples():
    amap[str(w.token_up)] = (w.slug, "up", w.t0_ms, w.end_ms)
    amap[str(w.token_down)] = (w.slug, "down", w.t0_ms, w.end_ms)
tops["key"] = tops.asset.astype(str)
tops = tops[tops.key.isin(amap)].copy()
tops["slug"] = tops.key.map(lambda k: amap[k][0])
tops["side"] = tops.key.map(lambda k: amap[k][1])
tops = tops.sort_values("recv_ms")

jumps = np.where((np.abs(ret) > 4 * sd) & (dt <= 2.0))[0] + 1  # indice du tick d'arrivée
print(f"\n== A. {len(jumps)} sauts >4σ ; réaction du meilleur ask du côté du saut ==")
rows = []
for j in jumps:
    t_j = rc[j]                                # heure de réception du saut chez nous
    side = "up" if ret[j - 1] > 0 else "down"
    # fenêtre active à cet instant
    m = [v for v in amap.values() if v[2] <= ts[j] < v[3] and v[1] == side]
    if not m:
        continue
    slug = m[0][0]
    tt = tops[(tops.slug == slug) & (tops.side == side)]
    before = tt[tt.recv_ms <= t_j].tail(1)
    if before.empty:
        continue
    a0 = float(before.ask.iloc[0])
    for delay in (250, 500, 1000, 2000, 5000):
        af = tt[tt.recv_ms <= t_j + delay].tail(1)
        rows.append((delay, float(af.ask.iloc[0]) - a0 if not af.empty else np.nan))
r = pd.DataFrame(rows, columns=["delay", "dask"]).dropna()
for d, g in r.groupby("delay"):
    print(f"  ask du côté du saut à +{d:>4}ms: variation médiane {g.dask.median():+.3f}, moyenne {g.dask.mean():+.3f} (n={len(g)})")

# ── B. Parité ───────────────────────────────────────────────────────────
print("\n== B. Parité ask_up+ask_down (achat 2 côtés) ==")
pu = tops[tops.side == "up"][["recv_ms", "slug", "ask"]].rename(columns={"ask": "au"})
pdn = tops[tops.side == "down"][["recv_ms", "slug", "ask"]].rename(columns={"ask": "ad"})
j = pd.merge_asof(pu.sort_values("recv_ms"), pdn.sort_values("recv_ms"),
                  on="recv_ms", by="slug", tolerance=3000, direction="backward").dropna()
j["s"] = j.au + j.ad
print(f"n={len(j)}  somme: p1={j.s.quantile(.01):.3f} p10={j.s.quantile(.10):.3f} "
      f"méd={j.s.median():.3f}  <0.99: {(j.s<0.99).mean()*100:.2f}%  <0.97: {(j.s<0.97).mean()*100:.3f}%")

# ── C. Momentum 1-5 s brut : le carnet laisse-t-il un edge directionnel ?
# après un saut, le prix continue-t-il ? (autocorr +0.24 à 1 s)
print("\n== C. Continuation après saut >4σ (oracle seul) ==")
for hor in (5, 15, 30):
    cont = []
    for jj in jumps:
        i2 = np.searchsorted(ts, ts[jj] + hor * 1000, "right") - 1
        if i2 > jj:
            cont.append(np.sign(lp[i2] - lp[jj]) == np.sign(ret[jj - 1]))
    if cont:
        print(f"  P(même direction après {hor:>2}s) = {np.mean(cont):.3f} (n={len(cont)})")
