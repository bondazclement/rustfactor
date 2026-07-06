#!/usr/bin/env python3
"""Étude quantitative du flux de résolution Chainlink + calibration du modèle.

Sortie : statistiques compactes sur stdout + analysis/out/moments.csv
(un enregistrement par (fenêtre, seconde) avec l'état du modèle et l'issue,
pour l'étape de re-modélisation).
"""
import numpy as np
import pandas as pd

OUT = "analysis/out"
HL = 60.0          # demi-vie EWMA (s) — défaut prod
DRIFT_W = 120.0    # fenêtre de drift (s) — défaut prod
DRIFT_CAP = 2.0    # plafond en unités de z — défaut prod

# ── Chargement ───────────────────────────────────────────────────────────
ticks = pd.read_csv(f"{OUT}/ticks.csv").sort_values("ts_ms").drop_duplicates("ts_ms")
ticks["s"] = ticks.ts_ms // 1000
win = pd.read_csv(f"{OUT}/windows.csv").sort_values("t0_ms")
tops = pd.read_csv(f"{OUT}/tops.csv")
print(f"ticks={len(ticks)} fenêtres={len(win)} tops={len(tops)}")

ts = ticks.ts_ms.to_numpy()
px = ticks.price.to_numpy()
lp = np.log(px)

# ── A. Nature du processus (rendements 1 s) ─────────────────────────────
dt = np.diff(ts) / 1000.0
ret = np.diff(lp)
m1 = (dt >= 0.5) & (dt <= 1.5)          # paires consécutives ~1 s
r1 = ret[m1]
sd = r1.std()
print("\n== A. Rendements 1 s ==")
print(f"n={len(r1)} sigma_1s={sd:.3e} (≈{sd*1e4:.2f} bp)  kurtosis_excès={pd.Series(r1).kurt():.1f}")
for k in (3, 5, 8):
    obs = (np.abs(r1) > k * sd).mean()
    from scipy.stats import norm
    theo = 2 * norm.sf(k)
    print(f"  P(|r|>{k}σ): observé {obs:.2e} vs gaussien {theo:.2e}  (×{obs/max(theo,1e-300):.0f})")
# Autocorrélation des rendements agrégés à h secondes
print("autocorr lag-1 des rendements agrégés (momentum>0 / réversion<0):")
s_ret = pd.Series(ret, index=pd.to_datetime(ts[1:], unit="ms")).resample("1s").sum()
for h in (1, 5, 15, 30, 60):
    a = s_ret.rolling(h).sum().dropna()[::h]
    print(f"  h={h:>3}s: ac1={a.autocorr(1):+.3f}")

# ── B. Simulation du modèle seconde par seconde ─────────────────────────
# EWMA de variance par seconde le long des ticks (identique à pm-core).
ew = np.empty(len(ts)); ew[:] = np.nan
v = np.nan
for i in range(1, len(ts)):
    d = (ts[i] - ts[i-1]) / 1000.0
    if d <= 0.01:
        ew[i] = v; continue
    vi = ret[i-1] ** 2 / d
    lam = np.exp(-np.log(2) * d / HL)
    v = vi if np.isnan(v) else lam * v + (1 - lam) * vi
    ew[i] = v
sig = np.sqrt(ew)  # σ par √s au tick i

rows = []
j0 = 0
for w in win.itertuples():
    t0, te = w.t0_ms, w.end_ms
    i_strike = np.searchsorted(ts, t0, "right") - 1
    i_final = np.searchsorted(ts, te, "right") - 1
    if i_strike < 0 or i_final <= i_strike:
        continue
    if ts[i_strike] < t0 - 2000 or ts[i_final] < te - 2000:
        continue                                    # couverture incomplète
    K, lK = px[i_strike], lp[i_strike]
    up_won = px[i_final] > K
    for tsec in range(t0 // 1000 + 10, te // 1000 - 2):
        tm = tsec * 1000
        i = np.searchsorted(ts, tm, "right") - 1
        if ts[i] < tm - 3000 or np.isnan(sig[i]) or sig[i] <= 0:
            continue
        tau = (te - tm) / 1000.0
        x = lp[i] - lK
        # drift 120 s (même méthode que prod : premier/dernier tick de la fenêtre)
        i_d = np.searchsorted(ts, tm - DRIFT_W * 1000, "left")
        mu = ((lp[i] - lp[i_d]) / ((ts[i] - ts[i_d]) / 1000.0)) if ts[i] > ts[i_d] else 0.0
        st = sig[i] * np.sqrt(tau)
        z_x = x / st
        z_mu = np.clip(mu * tau / st, -DRIFT_CAP, DRIFT_CAP)
        z = z_x + z_mu
        rows.append((w.slug, tsec - t0 // 1000, tau, K, px[i], x, sig[i], mu, z_x, z, up_won))
    j0 += 1
mom = pd.DataFrame(rows, columns=["slug", "elapsed", "tau", "K", "S", "x", "sig", "mu", "z_x", "z", "up_won"])
from scipy.stats import norm
mom["p_up"] = norm.cdf(mom.z)
mom.to_csv(f"{OUT}/moments.csv", index=False)
print(f"\n== B. Simulation modèle: {mom.slug.nunique()} fenêtres, {len(mom)} points ==")

# Calibration : p prédit vs fréquence réalisée (côté prédit)
mom["p_side"] = np.where(mom.p_up >= 0.5, mom.p_up, 1 - mom.p_up)
mom["side_won"] = np.where(mom.p_up >= 0.5, mom.up_won, ~mom.up_won.astype(bool))
print("calibration (tous instants):")
bins = [0.5, 0.8, 0.9, 0.95, 0.99, 0.995, 0.999, 1.0001]
g = mom.groupby(pd.cut(mom.p_side, bins), observed=True)
for b, gr in g:
    print(f"  p∈{str(b):>16}: n={len(gr):>6}  réalisé={gr.side_won.mean():.3f}")
print("calibration (τ ≤ 120 s uniquement):")
short = mom[mom.tau <= 120]
g = short.groupby(pd.cut(short.p_side, bins), observed=True)
for b, gr in g:
    print(f"  p∈{str(b):>16}: n={len(gr):>6}  réalisé={gr.side_won.mean():.3f}")

# ── C. Le marché sait-il mieux ? (jointure top-of-book) ─────────────────
tops["a"] = tops.asset.astype(str)
u = win.assign(a=win.token_up.astype(str))[["slug", "a"]].assign(side="up")
d = win.assign(a=win.token_down.astype(str))[["slug", "a"]].assign(side="down")
amap = pd.concat([u, d])
tops = tops.merge(amap, on="a", how="inner")
tops = tops.sort_values("recv_ms")
mom["tm"] = (mom.elapsed + mom.slug.str.rsplit("-", n=1).str[-1].astype(int)) * 1000
res = []
for side in ("up", "down"):
    t_s = tops[tops.side == side][["recv_ms", "slug", "ask", "bid"]]
    m_s = mom.reset_index()[["index", "slug", "tm"]]
    j = pd.merge_asof(m_s.sort_values("tm"), t_s.sort_values("recv_ms"),
                      left_on="tm", right_on="recv_ms", by="slug",
                      tolerance=8000, direction="backward")
    j = j.set_index("index")[["ask", "bid"]].rename(columns={"ask": f"ask_{side}", "bid": f"bid_{side}"})
    res.append(j)
mom = mom.join(res[0]).join(res[1])
have = mom.dropna(subset=["ask_up", "ask_down"])
# prob implicite marché pour Up : milieu du token up (via les deux carnets)
p_mkt = ((have.ask_up + have.bid_up) / 2 + (1 - (have.ask_down + have.bid_down) / 2)) / 2
have = have.assign(p_mkt=p_mkt)
b_model = ((have.p_up - have.up_won) ** 2).mean()
b_mkt = ((have.p_mkt - have.up_won) ** 2).mean()
b_mix = (((have.p_up + have.p_mkt) / 2 - have.up_won) ** 2).mean()
print(f"\n== C. Brier (n={len(have)}) ==  modèle={b_model:.4f}  marché={b_mkt:.4f}  mix50/50={b_mix:.4f}")
have.to_csv(f"{OUT}/moments_mkt.csv", index=False)

# ── D. Réversion en fin de fenêtre : P(flip | avance, τ) ────────────────
print("\n== D. P(le côté en tête à τ perd) par avance z_x (sans drift) ==")
for tlo, thi in ((5, 15), (15, 30), (30, 60), (60, 120), (120, 200)):
    sl = mom[(mom.tau >= tlo) & (mom.tau < thi)]
    for zlo, zhi in ((1.5, 2.5), (2.5, 3.5), (3.5, 6.0)):
        s2 = sl[(sl.z_x.abs() >= zlo) & (sl.z_x.abs() < zhi)]
        if len(s2) < 30:
            continue
        lead_won = np.where(s2.z_x > 0, s2.up_won, ~s2.up_won.astype(bool))
        print(f"  τ∈[{tlo:>3},{thi:>3})s |z_x|∈[{zlo},{zhi}): n={len(s2):>5} tête gagne={lead_won.mean():.3f} (modèle dirait {norm.cdf(s2.z_x.abs()).mean():.3f})")
