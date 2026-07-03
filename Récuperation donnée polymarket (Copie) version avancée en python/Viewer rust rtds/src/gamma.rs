use anyhow::{Context, Result};
use chrono::Utc;

use crate::models::{GammaEvent, MarketInfo};

const GAMMA_BASE: &str = "https://gamma-api.polymarket.com";

/// Calcule le timestamp du slot courant (arrondi aux 5 min)
pub fn current_slot_ts() -> u64 {
    let now = Utc::now().timestamp() as u64;
    (now / 300) * 300
}

/// Calcule le timestamp du prochain slot
pub fn next_slot_ts() -> u64 {
    let now = Utc::now().timestamp() as u64;
    ((now / 300) + 1) * 300
}

/// Construit le slug pour un timestamp de slot
pub fn slug_for_slot(ts: u64) -> String {
    format!("btc-updown-5m-{}", ts)
}

/// Récupère le marché btc-updown-5m actif pour le slot courant (ou le suivant si pas encore ouvert)
pub async fn fetch_active_market(client: &reqwest::Client) -> Result<MarketInfo> {
    // Essaie le slot courant, puis +1, puis +2
    for offset in 0u64..5 {
        let slot = current_slot_ts() + offset * 300;
        let slug = slug_for_slot(slot);
        match fetch_market_by_slug(client, &slug).await {
            Ok(Some(info)) => return Ok(info),
            Ok(None) => continue,
            Err(e) => {
                // Log mais on continue
                eprintln!("Error fetching {}: {}", slug, e);
                continue;
            }
        }
    }
    anyhow::bail!("No active btc-updown-5m market found for current slots")
}

async fn fetch_market_by_slug(
    client: &reqwest::Client,
    slug: &str,
) -> Result<Option<MarketInfo>> {
    let url = format!("{}/events?slug={}", GAMMA_BASE, slug);
    let resp: Vec<GammaEvent> = client
        .get(&url)
        .send()
        .await
        .context("HTTP request to Gamma failed")?
        .json()
        .await
        .context("Failed to parse Gamma response")?;

    let event = match resp.into_iter().next() {
        Some(e) => e,
        None => return Ok(None),
    };

    // Ignorer les marchés fermés
    if event.closed {
        return Ok(None);
    }

    let market = match event.markets.into_iter().next() {
        Some(m) => m,
        None => return Ok(None),
    };

    let token_ids = market.clob_token_ids();
    if token_ids.len() < 2 {
        return Ok(None);
    }

    // Vérifier que les outcomes sont bien Up/Down
    let outcomes = market.outcomes_list();
    let (token_up, token_down) = if outcomes.first().map(|s| s.as_str()) == Some("Up") {
        (token_ids[0].clone(), token_ids[1].clone())
    } else {
        (token_ids[1].clone(), token_ids[0].clone())
    };

    // Convertir endDate en timestamp ms
    let end_date_ms = chrono::DateTime::parse_from_rfc3339(&market.end_date)
        .map(|dt| dt.timestamp_millis() as u64)
        .unwrap_or(0);

    Ok(Some(MarketInfo {
        slug: event.slug,
        title: event.title,
        end_date_ms,
        condition_id: market.condition_id,
        token_up,
        token_down,
    }))
}
