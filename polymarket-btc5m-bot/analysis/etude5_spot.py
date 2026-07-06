#!/usr/bin/env python3
"""Étude 5 : le spot Binance (RTDS crypto_prices) précède-t-il l'oracle
Chainlink ? De combien ? Et surtout : à horizon court, connaître le spot
améliore-t-il la prédiction du print FINAL de l'oracle ?"""
import numpy as np
import pandas as pd

OUT = "analysis/out"
spot = pd.read_csv(f"{OUT}/spot.csv").sort_values("ts_ms")
tick = pd.read_csv(f"{OUT}/ticks.csv").sort_values("ts_ms")
# Restreindre l'oracle à la période couverte par le spot.
lo, hi = spot.ts_ms.min(), spot.ts_ms.max()
tk = tick[(tick.ts_ms >= lo) & (tick.ts_ms <= hi)].copy()
print(f"période commune: {(hi-lo)/3.6e6:.1f} h | spot n={len(spot)} "
      f"(cadence {np.median(np.diff(spot.ts_ms))/1000:.1f} s) | oracle n={len(tk)} "
      f"(cadence {np.median(np.diff(tk.ts_ms))/1000:.1f} s)")

# ── 1. Corrélation croisée des rendements (grille 1 s) ──────────────────
t0, t1 = lo, hi
grid = np.arange(t0, t1, 1000)
def serie(df):
    idx = np.searchsorted(df.ts_ms.to_numpy(), grid, "right") - 1
    idx = np.clip(idx, 0, len(df) - 1)
    return np.log(df.price.to_numpy()[idx])
ls, lt = serie(spot), serie(tk)
rs, rt = np.diff(ls), np.diff(lt)
# agrégation 5 s pour lisser la cadence du spot
k = 5
rs5 = rs[:len(rs)//k*k].reshape(-1, k).sum(1)
rt5 = rt[:len(rt)//k*k].reshape(-1, k).sum(1)
print("\ncorrélation croisée corr(r_spot(t−lag), r_oracle(t)) — pas de 5 s :")
best = (0, -1)
for lag in range(-4, 5):
    a = rs5[max(0,-lag):len(rs5)-max(0,lag)]
    b = rt5[max(0,lag):len(rt5)-max(0,-lag)]
    n = min(len(a), len(b)); a, b = a[:n], b[:n]
    c = np.corrcoef(a, b)[0, 1]
    m = " ←" if c > best[1] else ""
    if c > best[1]: best = (lag, c)
    print(f"  lag={lag*5:+3d}s : {c:+.3f}{m}")
print(f"→ le spot mène l'oracle de ~{best[0]*5} s (corr max {best[1]:.3f})" if best[0]>0
      else f"→ pas d'avance nette mesurable à cette cadence (max à lag {best[0]*5}s)")

# ── 2. Valeur prédictive incrémentale à horizon court ───────────────────
# À chaque seconde t avec τ = 10..60 s avant une frontière de 5 min :
# prédire le print oracle final. Baseline : dernier oracle connu.
# Question : quand |spot−oracle| diverge, qui a raison à la fin ?
print("\n== quand spot et oracle divergent de d $, le print oracle suivant va vers le spot ? ==")
si = spot.ts_ms.to_numpy(); sp = spot.price.to_numpy()
ti = tk.ts_ms.to_numpy(); tp = tk.price.to_numpy()
rows = []
for i in range(len(ti) - 1):
    # état à l'instant du tick oracle i : dernier spot connu
    j = np.searchsorted(si, ti[i], "right") - 1
    if j < 0 or si[j] < ti[i] - 8000:
        continue
    d = sp[j] - tp[i]                      # divergence spot − oracle
    nxt = tp[i + 1] - tp[i]                # mouvement du print oracle suivant
    rows.append((d, nxt))
df = pd.DataFrame(rows, columns=["div", "nxt"])
for dlo, dhi in ((5, 15), (15, 30), (30, 1e9)):
    e = df[df.div.abs().between(dlo, dhi)]
    if len(e) < 20:
        print(f"  |div|∈[{dlo},{dhi}): n={len(e)} (insuffisant)")
        continue
    agree = (np.sign(e.nxt) == np.sign(e.div)).mean()
    print(f"  |div|∈[{dlo:>3.0f},{dhi:>4.0f}): n={len(e):>4}  P(prochain print oracle va vers le spot)={agree:.3f}  "
          f"amplitude médiane {e.nxt.abs().median():.1f}$")
