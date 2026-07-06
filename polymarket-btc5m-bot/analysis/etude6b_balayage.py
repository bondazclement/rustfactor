#!/usr/bin/env python3
"""Étude 6b — balayage de stratégies « moment parfait ».
Simulation honnête : 1 entrée max/fenêtre, première seconde qualifiante,
frais réels, ½ tick de slippage. Pour CHAQUE règle (écart_min, τ_max,
plafond de prix) : trades, réussite, PnL, bornes de Wilson.
"""
import numpy as np
import pandas as pd

m = pd.read_csv("analysis/out/moments_mkt.csv")
m["side_up"] = m.z_x >= 0
m["ask_side"] = np.where(m.side_up, m.ask_up, m.ask_down)
m["side_won"] = np.where(m.side_up, m.up_won, ~m.up_won.astype(bool)).astype(bool)
m["dist"] = (np.exp(np.abs(m.x)) - 1) * m.K
h = m.dropna(subset=["ask_side"]).copy()
NW = h.slug.nunique(); HEURES = NW * 5 / 60
fee = lambda a: 0.07 * a * (1 - a)

def wilson(k, n, z):
    p = k / n; d = 1 + z*z/n; c = p + z*z/(2*n)
    s = z*np.sqrt(p*(1-p)/n + z*z/(4*n*n))
    return (c - s)/d

def run(dmin, tmax, cap, tmin=4.0):
    trades = []
    for slug, g in h.groupby("slug"):
        g = g.sort_values("elapsed")
        c = g[(g.dist >= dmin) & (g.tau <= tmax) & (g.tau >= tmin) & (g.ask_side <= cap)]
        if c.empty: continue
        e = c.iloc[0]
        a = min(e.ask_side + 0.005, 0.995)
        gain = 250/a*(1-a) - 250/a*fee(a)
        perte = -250 - 250/a*fee(a)
        trades.append((e.side_won, gain if e.side_won else perte, a))
    if not trades: return None
    df = pd.DataFrame(trades, columns=["won","pnl","prix"])
    n, k = len(df), int(df.won.sum())
    return dict(n=n, wr=k/n, plo=wilson(k,n,1.28), pnl=df.pnl.sum(),
                prix=df.prix.mean(), ph=n/HEURES,
                # seuil de rentabilité au prix moyen payé (frais inclus)
                seuil=(df.prix.mean()+fee(df.prix.mean()))/(1+0),
                )

print(f"corpus: {NW} fenêtres ≈ {HEURES:.1f} h")
print(f"{'écart':>6} {'τmax':>5} {'prix≤':>6} │ {'n':>3} {'/h':>4} {'réussite':>8} {'P_lo90':>6} {'seuil*':>6} {'PnL':>9} {'$/trade':>8}")
res = []
for dmin in (30, 40, 50, 70, 90):
    for tmax in (30, 60, 90, 120):
        for cap in (0.90, 0.94, 0.96, 0.98):
            r = run(dmin, tmax, cap)
            if not r or r["n"] < 5: continue
            res.append((dmin, tmax, cap, r))
            flag = " ◄" if r["plo"] > r["seuil"] else ""
            print(f"{dmin:>5}$ {tmax:>4}s {cap:>6.2f} │ {r['n']:>3} {r['ph']:>4.1f} {100*r['wr']:>7.1f}% {r['plo']:>6.3f} {r['seuil']:>6.3f} {r['pnl']:>+9.2f} {r['pnl']/r['n']:>+8.1f}{flag}")
print("* seuil = prix moyen payé + frais : il faut P(win) > seuil ; ◄ = prouvé à 90 % de confiance")

# top par PnL et par robustesse
res.sort(key=lambda x: -(x[3]["plo"] - x[3]["seuil"]))
print("\nTop 5 robustesse (P_lo90 − seuil) :")
for dmin, tmax, cap, r in res[:5]:
    print(f"  écart≥{dmin}$ τ≤{tmax}s prix≤{cap}: marge_lo={r['plo']-r['seuil']:+.3f} n={r['n']} PnL={r['pnl']:+.2f}$")
