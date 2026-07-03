use anyhow::{Context, Result};

use crate::models::{GammaEvent, MarketInfo};

const GAMMA_BASE: &str = "https://gamma-api.polymarket.com";

pub fn current_slot_ts() -> u64 {
    let now = chrono::Utc::now().timestamp() as u64;
    (now / 300) * 300
}

pub async fn fetch_active_market(client: &reqwest::Client) -> Result<MarketInfo> {
    for offset in 0..5u64 {
        let slot = current_slot_ts() + offset * 300;
        let slug = format!("btc-updown-5m-{}", slot);
        if let Some(m) = fetch_by_slug(client, &slug).await? {
            return Ok(m);
        }
    }
    anyhow::bail!("No active btc-updown-5m market found")
}

async fn fetch_by_slug(client: &reqwest::Client, slug: &str) -> Result<Option<MarketInfo>> {
    let url = format!("{}/events?slug={}", GAMMA_BASE, slug);
    let events: Vec<GammaEvent> = client
        .get(url)
        .send()
        .await
        .context("Gamma HTTP failed")?
        .json()
        .await
        .context("Gamma JSON parse failed")?;
    let Some(event) = events.into_iter().next() else {
        return Ok(None);
    };
    if event.closed {
        return Ok(None);
    }
    let Some(m) = event.markets.into_iter().next() else {
        return Ok(None);
    };
    let ids = m.clob_token_ids();
    if ids.len() < 2 {
        return Ok(None);
    }
    let outcomes = m.outcomes_list();
    let (token_up, token_down) = if outcomes.first().map(|s| s.as_str()) == Some("Up") {
        (ids[0].clone(), ids[1].clone())
    } else {
        (ids[1].clone(), ids[0].clone())
    };
    let end_date_ms = chrono::DateTime::parse_from_rfc3339(&m.end_date)
        .map(|dt| dt.timestamp_millis() as u64)
        .unwrap_or(0);
    Ok(Some(MarketInfo {
        slug: event.slug,
        title: event.title,
        end_date_ms,
        token_up,
        token_down,
    }))
}
