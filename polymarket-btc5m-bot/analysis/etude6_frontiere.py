#!/usr/bin/env python3
"""Étude 6 — LA LIGNE D'EFFICIENCE DU TRADER HUMAIN.

Trois questions :
1. Frontière : pour chaque horizon τ, à partir de quel écart $ le favori
   tient-il assez souvent pour battre le prix demandé + frais ?
2. Prix cible : dans chaque cellule (τ, écart), quel est le prix MAXIMAL
   rationnel à payer (P_win borne basse − frais − marge) et le carnet
   l'offre-t-il ?
3. Fréquence : combien d'opportunités par heure selon la marge de
   confiance exigée (le curseur du trader prudent → agressif) ?
"""
import numpy as np
import pandas as pd

m = pd.read_csv("analysis/out/moments_mkt.csv")
m["side_up"] = m.z_x >= 0
m["ask_side"] = np.where(m.side_up, m.ask_up, m.ask_down)
m["side_won"] = np.where(m.side_up, m.up_won, ~m.up_won.astype(bool)).astype(bool)
m["dist"] = (np.exp(np.abs(m.x)) - 1) * m.K
h = m.dropna(subset=["ask_side"]).copy()
NW = h.slug.nunique()
HEURES = NW * 5 / 60.0
print(f"{len(h)} états | {NW} fenêtres ≈ {HEURES:.1f} h de marché")

fee = lambda a: 0.07 * a * (1 - a)

def wilson_lo(k, n, z=1.28):  # borne basse ~90 %
    if n == 0: return 0.0
    p = k / n
    d = 1 + z * z / n
    c = p + z * z / (2 * n)
    s = z * np.sqrt(p * (1 - p) / n + z * z / (4 * n * n))
    return (c - s) / d

# ── 1. Grille fine + frontière ──────────────────────────────────────────
TB = [(4, 10), (10, 20), (20, 40), (40, 70), (70, 120), (120, 200), (200, 296)]
DBs = [15, 25, 40, 60, 90, 130]
print("\n══ P(win) borne basse 90 % │ ask médian │ EV nette au meilleur ask dispo ══")
print("écart↓ τ→   " + "".join(f"{a}-{b}s".center(19) for a, b in TB))
frontier = {}
for i, dmin in enumerate(DBs):
    dmax = DBs[i + 1] if i + 1 < len(DBs) else 1e9
    row = f"{dmin:>4}-{dmax if dmax<1e9 else '+':>4}$ "
    for tlo, thi in TB:
        e = h[(h.dist >= dmin) & (h.dist < dmax) & (h.tau >= tlo) & (h.tau < thi)]
        # dédupliquer par fenêtre (les secondes sont corrélées)
        g = e.groupby("slug").agg(won=("side_won", "max"), ask=("ask_side", "median"))
        n = len(g)
        if n < 8:
            row += "—".center(19); continue
        plo = wilson_lo(g.won.sum(), n)
        a = g.ask.median()
        ev = plo - a - fee(a)
        frontier[(dmin, tlo)] = (plo, a, ev, n)
        row += f"{plo:.2f}|{a:.2f}|{ev:+.2f} n={n:<3}".center(19)
    print(row)

# frontière ajustée : écart nécessaire ~ c·√τ (diffusion)
print("\n── Frontière ajustée (EV=0) : écart_min(τ) ≈ c·√τ ──")
pts = [(np.sqrt((tlo+thi)/2), dmin) for (dmin, tlo), (plo, a, ev, n) in frontier.items() if ev > 0
       for thi in [dict(TB)[tlo] if tlo in dict(TB) else tlo*2]]
if pts:
    cs = [d / s for s, d in pts]
    c_fit = np.percentile(cs, 25)  # bord bas de la zone positive
    print(f"  c ≈ {c_fit:.1f} $/√s → exemples : τ=25s → {c_fit*5:.0f} $ ; τ=60s → {c_fit*7.75:.0f} $ ; τ=100s → {c_fit*10:.0f} $")

# ── 2. Prix cible et disponibilité ──────────────────────────────────────
print("\n══ Le moment parfait : cellules EV>0, prix cible vs carnet ══")
for (dmin, tlo), (plo, a, ev, n) in sorted(frontier.items(), key=lambda kv: -kv[1][2]):
    if ev <= 0: continue
    prix_cible = plo - fee(a) - 0.01
    print(f"  écart≥{dmin:>3}$ τ∈[{tlo}..): P_win_lo={plo:.3f} → payer ≤{prix_cible:.3f} ; ask médian {a:.3f} ; EV {ev:+.3f} ({n} fen.)")

# ── 3. Fréquence vs marge de confiance ──────────────────────────────────
# règle : entrer si p_win_table(dist,τ) − ask − fee ≥ marge ; p par cellule (dédup fenêtre).
print("\n══ Courbe fréquence ↔ exigence (1 entrée max/fenêtre) ══")
cell_p = {}
for i, dmin in enumerate(DBs):
    dmax = DBs[i + 1] if i + 1 < len(DBs) else 1e9
    for tlo, thi in TB:
        e = h[(h.dist >= dmin) & (h.dist < dmax) & (h.tau >= tlo) & (h.tau < thi)]
        g = e.groupby("slug").side_won.max()
        if len(g) >= 8:
            cell_p[(i, tlo)] = wilson_lo(g.sum(), len(g))
def cell_of(d, t):
    i = np.searchsorted(DBs, d, "right") - 1
    for tlo, thi in TB:
        if tlo <= t < thi:
            return (i, tlo)
    return None
h["cell"] = [cell_of(d, t) for d, t in zip(h.dist, h.tau)]
h["p_cell"] = h.cell.map(cell_p)
hv = h.dropna(subset=["p_cell"])
for marge in (0.00, 0.01, 0.02, 0.04, 0.06):
    pnl, ntr, wins = 0.0, 0, 0
    for slug, g in hv.groupby("slug"):
        g = g.sort_values("elapsed")
        cand = g[(g.p_cell - g.ask_side - 0.07 * g.ask_side * (1 - g.ask_side)) >= marge]
        if cand.empty: continue
        e = cand.iloc[0]
        a = e.ask_side + 0.005
        sz = 250 / a
        pnl += sz * (1 - a) - sz * fee(a) if e.side_won else -250 - sz * fee(a)
        ntr += 1; wins += int(e.side_won)
    if ntr:
        print(f"  marge≥{marge:.2f}: {ntr:>3} trades ({ntr/HEURES:.1f}/h) réussite {100*wins/ntr:>5.1f}% PnL {pnl:+8.2f}$ ({pnl/ntr:+.1f}$/trade)")
    else:
        print(f"  marge≥{marge:.2f}: 0 trade")
