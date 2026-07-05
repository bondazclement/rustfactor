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

#[cfg(test)]
mod tests {
    use super::*;

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
