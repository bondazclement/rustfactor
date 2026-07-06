#!/usr/bin/env python3
"""Étude 2 : probabilité réelle CONDITIONNÉE aux moments d'entrée
(z fort ET ask du côté favori encore bas), ajustement Student-t,
et PnL simulé de règles de décision candidates."""
import numpy as np
import pandas as pd
from scipy.stats import norm, t as tdist
from scipy.optimize import minimize_scalar

OUT = "analysis/out"
mom = pd.read_csv(f"{OUT}/moments_mkt.csv")
mom["side_up"] = mom.z_x >= 0
mom["ask_side"] = np.where(mom.side_up, mom.ask_up, mom.ask_down)
mom["side_won"] = np.where(mom.side_up, mom.up_won, ~mom.up_won.astype(bool))
mom["az"] = mom.z_x.abs()
have = mom.dropna(subset=["ask_side"]).copy()
print(f"points avec carnet: {len(have)}, fenêtres {have.slug.nunique()}")

# ── 1. Ajustement Student-t sur les rendements 1 s ──────────────────────
ticks = pd.read_csv(f"{OUT}/ticks.csv").sort_values("ts_ms")
lp = np.log(ticks.price.to_numpy())
dt = np.diff(ticks.ts_ms.to_numpy()) / 1000.0
r1 = np.diff(lp)[(dt >= 0.5) & (dt <= 1.5)]
nu, loc, sc = tdist.fit(r1)
print(f"\n== Student-t sur r_1s: ν={nu:.2f} ==")
for z in (2.0, 2.5, 3.0, 4.0):
    print(f"  P(gagne|z={z}) : gaussien {norm.cdf(z):.4f} | t(ν={nu:.1f}) {tdist.cdf(z, nu):.4f}")

# ── 2. Calibration conditionnée à l'entrée (z≥seuil ET ask≤plafond) ────
print("\n== P(win) réelle aux moments d'entrée simulés ==")
print("(z_x sans drift ; ask = prix payé ; edge réel = P(win) − ask)")
for zmin in (2.0, 2.5, 3.0):
    for cap in (0.65, 0.75, 0.85, 0.95):
        e = have[(have.az >= zmin) & (have.az < zmin + 0.5) & (have.ask_side <= cap)
                 & (have.ask_side >= cap - 0.10) & (have.tau >= 10)]
        nw = e.slug.nunique()
        if len(e) < 40:
            continue
        print(f"  z∈[{zmin},{zmin+0.5}) ask∈({cap-0.10:.2f},{cap:.2f}]: n={len(e):>5} ({nw:>3} fen.) "
              f"P(win)={e.side_won.mean():.3f}  ask_moy={e.ask_side.mean():.3f}  edge={e.side_won.mean()-e.ask_side.mean():+.3f}")

# ── 3. Le désaccord marché/z comme variable : qui a raison ? ────────────
print("\n== P(win) par (z, prob implicite marché) — τ∈[20,180] ==")
sl = have[(have.tau >= 20) & (have.tau <= 180) & (have.az >= 2.0)]
pm = np.where(sl.side_up, sl.p_mkt, 1 - sl.p_mkt)  # prob marché du côté favori (z)
for plo, phi in ((0.4, 0.6), (0.6, 0.75), (0.75, 0.9), (0.9, 1.0)):
    e = sl[(pm >= plo) & (pm < phi)]
    if len(e) < 40:
        continue
    print(f"  p_mkt∈[{plo},{phi}): n={len(e):>5}  P(win côté z)={e.side_won.mean():.3f}  (z moyen {e.az.mean():.1f})")

# ── 4. Simulation de PnL par fenêtre : règles candidates ────────────────
# 1 entrée max par fenêtre (comme prod), notional 250, pas de sortie anticipée.
print("\n== PnL simulé (250$/trade, 1 trade max/fenêtre, coût 1c inclus) ==")
def simulate(df, rule, label):
    pnl, ntr, nw_ = 0.0, 0, 0
    for slug, g in df.groupby("slug"):
        g = g.sort_values("elapsed")
        cand = g[rule(g)]
        if cand.empty:
            continue
        e = cand.iloc[0]
        price = e.ask_side + 0.01
        size = 250.0 / price
        pnl += size * (1 - price) if e.side_won else -250.0
        ntr += 1
        nw_ += 1 if e.side_won else 0
    print(f"  {label:<58} trades={ntr:>3} wins={nw_:>3} PnL={pnl:+8.2f}")

emp = {}  # table empirique P(win | z-bin, τ-bin) apprise sur les données
zb = [2.0, 2.5, 3.5, 9.9]; tb = [10, 30, 60, 120, 200, 292]
for i in range(len(zb) - 1):
    for j in range(len(tb) - 1):
        m = (have.az >= zb[i]) & (have.az < zb[i+1]) & (have.tau >= tb[j]) & (have.tau < tb[j+1])
        emp[(i, j)] = have[m].side_won.mean() if m.sum() >= 50 else np.nan
def p_emp(g):
    zi = np.clip(np.digitize(g.az, zb) - 1, 0, len(zb) - 2)
    ti = np.clip(np.digitize(g.tau, tb) - 1, 0, len(tb) - 2)
    return np.array([emp.get((a, b), np.nan) for a, b in zip(zi, ti)])

simulate(have, lambda g: (g.az >= 2.5) & (g.ask_side <= 0.74) & (g.tau >= 10),
         "PROD actuelle: z≥2.5, ask≤0.75")
simulate(have, lambda g: (g.az >= 2.5) & (g.ask_side <= 0.74) & (g.tau >= 30),
         "PROD + τ≥30s")
simulate(have, lambda g: (p_emp(g) - (g.ask_side + 0.01) >= 0.08) & (g.tau >= 30) & (g.az >= 2.0),
         "EV empirique: p_table − prix ≥ 0.08, τ≥30")
simulate(have, lambda g: (p_emp(g) - (g.ask_side + 0.01) >= 0.12) & (g.tau >= 30) & (g.az >= 2.0),
         "EV empirique ≥ 0.12, τ≥30")
pmix = lambda g: 0.5 * p_emp(g) + 0.5 * np.where(g.side_up, g.p_mkt, 1 - g.p_mkt)
simulate(have, lambda g: (pmix(g) - (g.ask_side + 0.01) >= 0.08) & (g.tau >= 30) & (g.az >= 2.0),
         "EV mix(table,marché): ≥ 0.08, τ≥30")
simulate(have, lambda g: (tdist.cdf(g.az, nu) - (g.ask_side + 0.01) >= 0.10) & (g.tau >= 30) & (g.az >= 2.0),
         "EV Student-t(ν fitté): p_t − prix ≥ 0.10, τ≥30")
