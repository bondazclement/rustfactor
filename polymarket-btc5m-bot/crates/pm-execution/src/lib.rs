//! pm-execution — module 4 : transmission des ordres au CLOB Polymarket.
//!
//! Chemin le plus direct possible : signature EIP-712 locale + POST vers
//! `https://clob.polymarket.com` via le SDK Rust officiel
//! (`polymarket_client_sdk_v2`, feature `live`) — aucune couche
//! intermédiaire, aucun proxy applicatif.
//!
//! Par défaut le crate compile SANS le SDK : la passerelle `DryRun` journalise
//! les ordres qu'elle AURAIT envoyés (audit NDJSON), ce qui permet le paper
//! trading et les tests hors ligne. La passerelle live s'active avec
//! `--features live` + les variables d'environnement :
//!   POLYMARKET_PRIVATE_KEY  (clé du signer)
//!   POLYMARKET_FUNDER       (adresse qui détient les fonds, selon le type de
//!                            signature choisi — voir docs authentication).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderSide {
    Buy,
    Sell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimeInForce {
    /// Good-till-cancelled — ordres maker au repos.
    Gtc,
    /// Fill-or-kill — taker tout-ou-rien.
    Fok,
    /// Fill-and-kill / IOC — taker partiel accepté.
    Fak,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OrderRequest {
    pub token_id: String,
    pub side: OrderSide,
    pub price: f64,
    pub size: f64,
    pub tif: TimeInForce,
    /// Étiquette de la stratégie émettrice (audit).
    pub tag: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OrderAck {
    pub order_id: String,
    pub accepted: bool,
    pub detail: String,
}

/// Passerelle d'ordres. Implémentations : `DryRunGateway` (défaut) et
/// `LiveGateway` (feature `live`).
pub trait OrderGateway: Send + Sync {
    fn post_order(
        &self,
        req: OrderRequest,
    ) -> impl std::future::Future<Output = anyhow::Result<OrderAck>> + Send;
    fn cancel_all(
        &self,
        token_id: &str,
    ) -> impl std::future::Future<Output = anyhow::Result<()>> + Send;
}

/// Paper trading : accepte tout, journalise tout.
#[derive(Debug, Default)]
pub struct DryRunGateway {
    counter: std::sync::atomic::AtomicU64,
}

impl DryRunGateway {
    pub fn new() -> Self {
        Self::default()
    }
}

impl OrderGateway for DryRunGateway {
    async fn post_order(&self, req: OrderRequest) -> anyhow::Result<OrderAck> {
        let n = self
            .counter
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let order_id = format!("dryrun-{n}");
        tracing::info!(
            target: "execution",
            "[DRY-RUN] {} {:?} {} x {:.0} @ {:.3} ({:?})",
            order_id,
            req.side,
            &req.token_id[..8.min(req.token_id.len())],
            req.size,
            req.price,
            req.tif
        );
        Ok(OrderAck {
            order_id,
            accepted: true,
            detail: format!("dry-run {}", req.tag),
        })
    }

    async fn cancel_all(&self, token_id: &str) -> anyhow::Result<()> {
        tracing::info!(target: "execution", "[DRY-RUN] cancel_all {}", &token_id[..8.min(token_id.len())]);
        Ok(())
    }
}

/// Passerelle réelle via le SDK officiel. Squelette volontairement minimal :
/// il suit mot à mot le quickstart de la doc (client authentifié L1→L2 puis
/// `limit_order().build()` → `sign()` → `post_order()`). À activer et tester
/// sur un environnement avec accès réseau à clob.polymarket.com.
#[cfg(feature = "live")]
pub mod live {
    // use polymarket_client_sdk_v2::auth::{LocalSigner, Signer};
    // use polymarket_client_sdk_v2::clob::{Client, Config};
    // use polymarket_client_sdk_v2::clob::types::Side;
    // use polymarket_client_sdk_v2::types::dec;
    //
    // Implémentation prévue (cf. docs /quickstart + /trading/orders/create) :
    //   let signer = LocalSigner::from_str(&private_key)?.with_chain_id(Some(POLYGON));
    //   let client = Client::new("https://clob.polymarket.com", Config::default())?
    //       .authentication_builder(&signer).authenticate().await?;
    //   let order = client.limit_order().token_id(id).price(dec!(p)).size(dec!(s))
    //       .side(side).build().await?;
    //   let signed = client.sign(&signer, order).await?;
    //   client.post_order(signed).await?;
    //
    // Le SDK gère nativement tick size / neg_risk / fee rate (auto-fetch), ce
    // qui élimine une source d'erreur et un aller-retour réseau.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn dry_run_acks_and_numbers_orders() {
        let gw = DryRunGateway::new();
        let req = OrderRequest {
            token_id: "1234567890".into(),
            side: OrderSide::Buy,
            price: 0.62,
            size: 50.0,
            tif: TimeInForce::Fok,
            tag: "taker".into(),
        };
        let a1 = gw.post_order(req.clone()).await.unwrap();
        let a2 = gw.post_order(req).await.unwrap();
        assert!(a1.accepted && a2.accepted);
        assert_ne!(a1.order_id, a2.order_id);
        gw.cancel_all("1234567890").await.unwrap();
    }
}
