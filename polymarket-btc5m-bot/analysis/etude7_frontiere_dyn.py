#!/usr/bin/env python3
"""Étude 7 — paramétrisation finale du modèle v4.
A) Frontière en c·√τ : balayage du coefficient (le curseur de confiance).
B) Sortie dynamique : quand le favori acheté se retourne (écart franchit
   ~0), que récupère une vente au bid vs porter jusqu'au règlement ?
"""
import numpy as np
import pandas as pd

m = pd.read_csv("analysis/out/moments_mkt.csv")
m["side_up"] = m.z_x >= 0
m["ask_side"] = np.where(m.side_up, m.ask_up, m.ask_down)
m["bid_side"] = np.where(m.side_up, m.bid_up, m.bid_down)
m["side_won"] = np.where(m.side_up, m.up_won, ~m.up_won.astype(bool)).astype(bool)
m["dist"] = (np.exp(np.abs(m.x)) - 1) * m.K
m["dist_signee"] = np.sign(m.x) * m.dist
h = m.dropna(subset=["ask_side"]).copy()
NW = h.slug.nunique(); HEURES = NW * 5 / 60
fee = lambda a: 0.07 * a * (1 - a)
print(f"corpus: {NW} fenêtres ≈ {HEURES:.1f} h")

def wilson_lo(k, n, z=1.28):
    p = k/n; d = 1+z*z/n; c = p+z*z/(2*n)
    s = z*np.sqrt(p*(1-p)/n + z*z/(4*n*n))
    return (c-s)/d

# ── A. Frontière c·√τ ────────────────────────────────────────────────────
print("\n══ A. Entrée si écart ≥ max(dist_min, c·√τ), τ∈[4,τmax], ask ≤ cap ══")
print(f"{'c':>4} {'τmax':>5} {'cap':>5} {'d_min':>5} │ {'n':>3} {'/h':>4} {'réuss.':>7} {'P_lo':>5} {'seuil':>5} {'PnL':>8}")
best = []
for c in (4.0, 5.0, 6.0, 7.0, 8.0):
    for tmax in (90, 120, 150):
        for cap in (0.96, 0.98):
            for dmin in (30,):
                trades = []
                for slug, g in h.groupby("slug"):
                    g = g.sort_values("elapsed")
                    q = g[(g.dist >= np.maximum(dmin, c*np.sqrt(g.tau))) & (g.tau <= tmax)
                          & (g.tau >= 4) & (g.ask_side <= cap)]
                    if q.empty: continue
                    e = q.iloc[0]
                    a = min(e.ask_side + 0.005, 0.995)
                    pnl = 250/a*(1-a) - 250/a*fee(a) if e.side_won else -250 - 250/a*fee(a)
                    trades.append((e.side_won, pnl, a, slug, e.elapsed))
                if len(trades) < 8: continue
                df = pd.DataFrame(trades, columns=["won","pnl","prix","slug","el"])
                n, k = len(df), int(df.won.sum())
                plo = wilson_lo(k, n); pm = df.prix.mean(); seuil = pm + fee(pm)
                best.append((plo-seuil, c, tmax, cap, df))
                mk = " ◄" if plo > seuil else ""
                print(f"{c:>4.1f} {tmax:>4}s {cap:>5.2f} {dmin:>4}$ │ {n:>3} {n/HEURES:>4.1f} {100*k/n:>6.1f}% {plo:>5.3f} {seuil:>5.3f} {df.pnl.sum():>+8.1f}{mk}")

# ── B. Valeur du stop (sur la meilleure variante ◄) ─────────────────────
best.sort(key=lambda x: -x[0])
_, c, tmax, cap, df = best[0]
print(f"\n══ B. Sortie dynamique sur la variante c={c}, τmax={tmax}, cap={cap} ══")
tot_sans, tot_avec, n_stops = 0.0, 0.0, 0
for _, tr in df.iterrows():
    g = h[h.slug == tr.slug].sort_values("elapsed")
    ent = g[g.elapsed >= tr.el].iloc[0]
    side_up_pos = ent.side_up
    apres = g[g.elapsed > tr.el]
    # retournement : l'écart signé passe à ≥ 15 $ CONTRE la position
    contre = apres[np.where(side_up_pos, -apres.dist_signee, apres.dist_signee) >= 15.0]
    pnl_sans = tr.pnl
    if not contre.empty:
        srt = contre.iloc[0]
        bid = srt.bid_up if side_up_pos else srt.bid_down
        if not np.isnan(bid) and bid > 0.02:
            a = tr.prix
            sz = 250/a
            px_v = max(bid - 0.01, 0.01)
            pnl_avec = sz*(px_v - a) - sz*fee(a) - sz*fee(px_v)
            n_stops += 1
        else:
            pnl_avec = pnl_sans
    else:
        pnl_avec = pnl_sans
    tot_sans += pnl_sans; tot_avec += pnl_avec
print(f"  {len(df)} trades, {n_stops} stops déclenchés")
print(f"  PnL sans stop : {tot_sans:+.2f} $ | avec stop (retournement 15 $, vente au bid) : {tot_avec:+.2f} $")
