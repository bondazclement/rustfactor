//! Découverte des fenêtres via l'API Gamma.
//!
//! Les slugs sont déterministes (`btc-updown-5m-<epoch>` avec epoch multiple
//! de 300), donc la découverte interroge directement le slot courant et les
//! suivants — pas de scan de liste, pas de dépendance à un ordre de tri.

use crate::GAMMA_BASE_URL;
use anyhow::{Context, Result};
use pm_core::{window, MarketWindow};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
struct GammaEvent {
    slug: String,
    #[serde(default)]
    closed: bool,
    #[serde(default)]
    markets: Vec<GammaMarket>,
}

#[derive(Debug, Deserialize)]
struct GammaMarket {
    #[serde(rename = "conditionId", default)]
    condition_id: String,
    #[serde(rename = "clobTokenIds", default)]
    clob_token_ids_raw: String,
    #[serde(default)]
    outcomes: String,
    #[serde(rename = "negRisk", default)]
    neg_risk: bool,
    #[serde(rename = "orderPriceMinTickSize", default)]
    tick_size: Option<Value>,
}

pub struct GammaClient {
    http: reqwest::Client,
    base: String,
}

impl GammaClient {
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            http,
            base: GAMMA_BASE_URL.to_string(),
        }
    }

    pub fn with_base(http: reqwest::Client, base: impl Into<String>) -> Self {
        Self {
            http,
            base: base.into(),
        }
    }

    /// Fenêtre active au temps `now_s`, sinon la prochaine ouverte (jusqu'à
    /// `lookahead` slots en avant).
    pub async fn find_active_window(&self, now_s: u64, lookahead: u64) -> Result<MarketWindow> {
        let base_epoch = window::slot_epoch(now_s);
        for k in 0..=lookahead {
            let epoch = base_epoch + k * window::WINDOW_SECS;
            if let Some(w) = self.fetch_window(epoch).await? {
                return Ok(w);
            }
        }
        anyhow::bail!("aucune fenêtre btc-updown-5m ouverte trouvée (base={base_epoch})")
    }

    pub async fn fetch_window(&self, epoch_s: u64) -> Result<Option<MarketWindow>> {
        let slug = window::slug_for(epoch_s);
        let url = format!("{}/events?slug={}", self.base, slug);
        let events: Vec<GammaEvent> = self
            .http
            .get(&url)
            .send()
            .await
            .context("requête Gamma")?
            .error_for_status()
            .context("statut Gamma")?
            .json()
            .await
            .context("JSON Gamma")?;
        Ok(events
            .into_iter()
            .next()
            .and_then(|e| parse_event(e, epoch_s)))
    }
}

fn parse_event(event: GammaEvent, epoch_s: u64) -> Option<MarketWindow> {
    if event.closed {
        return None;
    }
    let market = event.markets.into_iter().next()?;
    let ids: Vec<String> = serde_json::from_str(&market.clob_token_ids_raw).unwrap_or_default();
    if ids.len() < 2 {
        return None;
    }
    let outcomes: Vec<String> = serde_json::from_str(&market.outcomes).unwrap_or_default();
    // Associer chaque token à son outcome par POSITION (jamais par supposition).
    let (token_up, token_down) = if outcomes.first().map(String::as_str) == Some("Down") {
        (ids[1].clone(), ids[0].clone())
    } else {
        // "Up" en premier — cas nominal observé et vérifié par le legacy.
        (ids[0].clone(), ids[1].clone())
    };
    let tick_size = match &market.tick_size {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.01),
        Some(Value::String(s)) => s.parse().unwrap_or(0.01),
        _ => 0.01,
    };
    let (start_ms, end_ms) = window::window_bounds_ms(epoch_s);
    Some(MarketWindow {
        slug: event.slug,
        epoch_s,
        start_ms,
        end_ms,
        condition_id: market.condition_id,
        token_up,
        token_down,
        neg_risk: market.neg_risk,
        tick_size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event_json(outcomes: &str) -> GammaEvent {
        serde_json::from_value(serde_json::json!({
            "slug": "btc-updown-5m-1778341500",
            "closed": false,
            "markets": [{
                "conditionId": "0xabc",
                "clobTokenIds": "[\"111\",\"222\"]",
                "outcomes": outcomes,
                "negRisk": false,
                "orderPriceMinTickSize": 0.01
            }]
        }))
        .unwrap()
    }

    #[test]
    fn token_mapping_up_first() {
        let w = parse_event(event_json("[\"Up\",\"Down\"]"), 1_778_341_500).unwrap();
        assert_eq!(w.token_up, "111");
        assert_eq!(w.token_down, "222");
        assert_eq!(w.start_ms, 1_778_341_500_000);
        assert_eq!(w.end_ms, 1_778_341_800_000);
    }

    #[test]
    fn token_mapping_down_first() {
        let w = parse_event(event_json("[\"Down\",\"Up\"]"), 1_778_341_500).unwrap();
        assert_eq!(w.token_up, "222");
        assert_eq!(w.token_down, "111");
    }

    #[test]
    fn closed_event_is_none() {
        let mut e = event_json("[\"Up\",\"Down\"]");
        e.closed = true;
        assert!(parse_event(e, 1_778_341_500).is_none());
    }
}
