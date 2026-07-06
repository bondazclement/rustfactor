//! Petites primitives mathématiques (pas de dépendance externe).

/// Fonction d'erreur, approximation d'Abramowitz & Stegun 7.1.26
/// (erreur absolue < 1.5e-7 — largement suffisant pour des probabilités de pari).
pub fn erf(x: f64) -> f64 {
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let x = x.abs();
    let t = 1.0 / (1.0 + 0.3275911 * x);
    let y = 1.0
        - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t
            + 0.254829592)
            * t
            * (-x * x).exp();
    sign * y
}

/// CDF de la loi normale standard.
pub fn norm_cdf(x: f64) -> f64 {
    0.5 * (1.0 + erf(x / std::f64::consts::SQRT_2))
}

/// ln Γ(x) — approximation de Lanczos (précision ~1e-10, x > 0).
fn ln_gamma(x: f64) -> f64 {
    const G: [f64; 6] = [
        76.180_091_729_471_46,
        -86.505_320_329_416_77,
        24.014_098_240_830_91,
        -1.231_739_572_450_155,
        0.120_865_097_386_617_9e-2,
        -0.539_523_938_495_3e-5,
    ];
    let mut ser = 1.000_000_000_190_015;
    let mut den = x;
    for g in G {
        den += 1.0;
        ser += g / den;
    }
    let tmp = x + 5.5;
    (2.506_628_274_631_000_5 * ser / x).ln() - tmp + (x + 0.5) * tmp.ln()
}

/// Fraction continue de la bêta incomplète (Numerical Recipes `betacf`).
fn betacf(a: f64, b: f64, x: f64) -> f64 {
    const MAX_IT: usize = 200;
    const EPS: f64 = 3e-14;
    const FPMIN: f64 = 1e-300;
    let qab = a + b;
    let qap = a + 1.0;
    let qam = a - 1.0;
    let mut c = 1.0;
    let mut d = 1.0 - qab * x / qap;
    if d.abs() < FPMIN {
        d = FPMIN;
    }
    d = 1.0 / d;
    let mut h = d;
    for m in 1..=MAX_IT {
        let m = m as f64;
        let m2 = 2.0 * m;
        let aa = m * (b - m) * x / ((qam + m2) * (a + m2));
        d = 1.0 + aa * d;
        if d.abs() < FPMIN {
            d = FPMIN;
        }
        c = 1.0 + aa / c;
        if c.abs() < FPMIN {
            c = FPMIN;
        }
        d = 1.0 / d;
        h *= d * c;
        let aa = -(a + m) * (qab + m) * x / ((a + m2) * (qap + m2));
        d = 1.0 + aa * d;
        if d.abs() < FPMIN {
            d = FPMIN;
        }
        c = 1.0 + aa / c;
        if c.abs() < FPMIN {
            c = FPMIN;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < EPS {
            break;
        }
    }
    h
}

/// Bêta incomplète régularisée I_x(a, b).
pub fn inc_beta(a: f64, b: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let bt = (ln_gamma(a + b) - ln_gamma(a) - ln_gamma(b) + a * x.ln() + b * (1.0 - x).ln()).exp();
    if x < (a + 1.0) / (a + b + 2.0) {
        bt * betacf(a, b, x) / a
    } else {
        1.0 - bt * betacf(b, a, 1.0 - x) / b
    }
}

/// CDF de la loi de Student à `nu` degrés de liberté (queues épaisses).
///
/// Pour la même distance z, elle donne une probabilité bien moins extrême
/// que la gaussienne — mesuré sur les ticks Chainlink : kurtosis ≈ 238,
/// P(|r|>5σ) ≈ 10⁴× le taux gaussien (docs/ETUDE_MODELE.md).
pub fn student_t_cdf(t: f64, nu: f64) -> f64 {
    if !t.is_finite() {
        return if t > 0.0 { 1.0 } else { 0.0 };
    }
    let nu = nu.max(0.1);
    let x = nu / (nu + t * t);
    let p = 0.5 * inc_beta(nu / 2.0, 0.5, x);
    if t >= 0.0 {
        1.0 - p
    } else {
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn student_t_reference_values() {
        // Références scipy.stats.t.cdf
        assert!((student_t_cdf(0.0, 2.0) - 0.5).abs() < 1e-9);
        assert!((student_t_cdf(1.0, 1.0) - 0.75).abs() < 1e-6);
        assert!((student_t_cdf(2.5, 2.0) - 0.935194).abs() < 1e-5);
        assert!((student_t_cdf(3.0, 2.0) - 0.952267).abs() < 1e-5);
        assert!((student_t_cdf(2.0, 4.0) - 0.941941).abs() < 1e-5);
        assert!((student_t_cdf(-2.5, 2.0) - (1.0 - 0.935194)).abs() < 1e-5);
        // Monotone en z, symétrique.
        assert!(student_t_cdf(3.0, 3.0) > student_t_cdf(2.0, 3.0));
        // Les queues t sont plus lourdes : p(z) < Φ(z) pour z grand.
        assert!(student_t_cdf(3.0, 2.0) < norm_cdf(3.0));
    }

    #[test]
    fn norm_cdf_reference_values() {
        assert!((norm_cdf(0.0) - 0.5).abs() < 1e-9);
        assert!((norm_cdf(1.0) - 0.841344746).abs() < 1e-6);
        assert!((norm_cdf(-1.0) - 0.158655254).abs() < 1e-6);
        assert!((norm_cdf(1.959964) - 0.975).abs() < 1e-5);
        assert!(norm_cdf(8.0) > 0.999999);
        assert!(norm_cdf(-8.0) < 1e-6);
    }
}
