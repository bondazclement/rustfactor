#!/usr/bin/env python3
"""Étude 4 : le « trade évident du trader humain ».
Croisement : écart spot↔strike EN DOLLARS × temps restant × prix demandé.
P(le favori gagne) mesurée, edge net après frais taker, simulation d'entrées.
"""
import numpy as np
import pandas as pd

m = pd.read_csv("analysis/out/moments_mkt.csv")
m["side_up"] = m.z_x >= 0
m["ask_side"] = np.where(m.side_up, m.ask_up, m.ask_down)
m["side_won"] = np.where(m.side_up, m.up_won, ~m.up_won.astype(bool)).astype(bool)
m["dist_usd"] = (np.exp(np.abs(m.x)) - 1) * m.K  # |S−K| en dollars
h = m.dropna(subset=["ask_side"]).copy()
print(f"{len(h)} états, {h.slug.nunique()} fenêtres | BTC ~{h.K.mean():,.0f} $")

fee = lambda a: 0.07 * a * (1 - a)

# ── Grille écart $ × temps restant ──────────────────────────────────────
DB = [(20, 50), (50, 100), (100, 200), (200, 9999)]
TB = [(3, 15), (15, 30), (30, 60), (60, 120), (120, 300)]
print("\n══ P(favori gagne) | ask moyen achetable | edge net après frais ══")
print(f"{'écart $':>10} │ " + " │ ".join(f"τ {a}-{b}s".center(24) for a, b in TB))
for dlo, dhi in DB:
    row = f"{dlo}-{dhi if dhi<9999 else '+':>4} $ │ "
    cells = []
    for tlo, thi in TB:
        e = h[(h.dist_usd >= dlo) & (h.dist_usd < dhi) & (h.tau >= tlo) & (h.tau < thi)]
        if len(e) < 40:
            cells.append("—".center(24))
            continue
        pw, a = e.side_won.mean(), e.ask_side.mean()
        edge = pw - a - fee(a)
        cells.append(f"{pw:.3f} a={a:.3f} e={edge:+.3f}".center(24))
    print(row + " │ ".join(cells))

# ── Le même tableau, RESTREINT aux états encore achetables (ask ≤ 0.97) ─
print("\n══ Restreint aux états ACHETABLES (ask ≤ 0.97) — le trade humain ══")
print(f"{'écart $':>10} │ " + " │ ".join(f"τ {a}-{b}s".center(28) for a, b in TB))
ha = h[h.ask_side <= 0.97]
for dlo, dhi in DB:
    row = f"{dlo}-{dhi if dhi<9999 else '+':>4} $ │ "
    cells = []
    for tlo, thi in TB:
        e = ha[(ha.dist_usd >= dlo) & (ha.dist_usd < dhi) & (ha.tau >= tlo) & (ha.tau < thi)]
        nw = e.slug.nunique()
        if len(e) < 40:
            cells.append("—".center(28))
            continue
        pw, a = e.side_won.mean(), e.ask_side.mean()
        edge = pw - a - fee(a)
        cells.append(f"{pw:.3f} a={a:.3f} e={edge:+.3f} f={nw}".center(28))
    print(row + " │ ".join(cells))

# ── Simulation du trade humain : 1 entrée max/fenêtre, 250 $, frais inclus ─
print("\n══ Simulation : entrer quand écart ≥ D$ ET τ ≤ T ET ask ≤ cap ══")
def sim(dmin, tmax, cap):
    pnl, ntr, wins = 0.0, 0, 0
    for slug, g in h.groupby("slug"):
        g = g.sort_values("elapsed")
        c = g[(g.dist_usd >= dmin) & (g.tau <= tmax) & (g.tau >= 4) & (g.ask_side <= cap)]
        if c.empty:
            continue
        e = c.iloc[0]
        a = e.ask_side + 0.005  # ½ tick de slippage
        size = 250.0 / a
        pnl += size * (1 - a) - size * fee(a) if e.side_won else -250.0 - size * fee(a)
        ntr += 1
        wins += int(e.side_won)
    br = f"{100*wins/ntr:.1f}%" if ntr else "—"
    print(f"  écart≥{dmin:>3}$ τ≤{tmax:>3}s ask≤{cap}: trades={ntr:>3} réussite={br:>6} PnL={pnl:+9.2f}$")

for dmin in (50, 100, 150):
    for tmax, cap in ((30, 0.97), (60, 0.97), (60, 0.90), (120, 0.85)):
        sim(dmin, tmax, cap)
