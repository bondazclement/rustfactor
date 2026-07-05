//! Arithmétique des fenêtres btc-updown-5m.
//!
//! Un marché `btc-updown-5m-<epoch>` couvre [epoch, epoch+300) secondes UTC.
//! L'epoch du slug est toujours un multiple de 300. La frontière T0 (= epoch)
//! définit le « price to beat » ; la frontière T0+300 définit la résolution.

pub const WINDOW_SECS: u64 = 300;
pub const SLUG_PREFIX: &str = "btc-updown-5m-";

/// Epoch (secondes) de la fenêtre contenant l'instant `now_s`.
pub fn slot_epoch(now_s: u64) -> u64 {
    (now_s / WINDOW_SECS) * WINDOW_SECS
}

pub fn slug_for(epoch_s: u64) -> String {
    format!("{SLUG_PREFIX}{epoch_s}")
}

/// Extrait l'epoch d'un slug `btc-updown-5m-<epoch>` (ou d'une URL le contenant).
pub fn epoch_from_slug(slug: &str) -> Option<u64> {
    let tail = slug.rsplit('/').next()?;
    let tail = tail.split(['?', '#']).next()?;
    let epoch: u64 = tail.rsplit('-').next()?.parse().ok()?;
    epoch.is_multiple_of(WINDOW_SECS).then_some(epoch)
}

/// Bornes de la fenêtre en millisecondes : [start_ms, end_ms).
pub fn window_bounds_ms(epoch_s: u64) -> (u64, u64) {
    (epoch_s * 1000, (epoch_s + WINDOW_SECS) * 1000)
}

/// Temps restant avant résolution, en ms (0 si dépassé).
pub fn remaining_ms(epoch_s: u64, now_ms: u64) -> u64 {
    window_bounds_ms(epoch_s).1.saturating_sub(now_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_alignment() {
        assert_eq!(slot_epoch(1_778_341_500), 1_778_341_500);
        assert_eq!(slot_epoch(1_778_341_799), 1_778_341_500);
        assert_eq!(slot_epoch(1_778_341_800), 1_778_341_800);
    }

    #[test]
    fn slug_roundtrip() {
        let slug = slug_for(1_778_341_500);
        assert_eq!(slug, "btc-updown-5m-1778341500");
        assert_eq!(epoch_from_slug(&slug), Some(1_778_341_500));
        assert_eq!(
            epoch_from_slug("https://polymarket.com/event/btc-updown-5m-1778343900?tid=1"),
            Some(1_778_343_900)
        );
        assert_eq!(epoch_from_slug("btc-updown-5m-1778341501"), None); // pas multiple de 300
        assert_eq!(epoch_from_slug("garbage"), None);
    }

    #[test]
    fn bounds() {
        let (s, e) = window_bounds_ms(1_778_341_500);
        assert_eq!(s, 1_778_341_500_000);
        assert_eq!(e, 1_778_341_800_000);
        assert_eq!(remaining_ms(1_778_341_500, s + 280_000), 20_000);
        assert_eq!(remaining_ms(1_778_341_500, e + 1), 0);
    }
}
