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
    /// Parts réellement exécutées (réponse du CLOB). En paper : la taille
    /// demandée. 0 = ordre accepté mais non exécuté (FAK tué) — leçon du
    /// micro-test du 06/07 : 1 décision sur 3 seulement était exécutée.
    pub taille_executee: f64,
    /// Prix moyen réel payé (USDC/part) quand exécuté.
    pub prix_reel: Option<f64>,
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
            taille_executee: req.size,
            prix_reel: Some(req.price),
        })
    }

    async fn cancel_all(&self, token_id: &str) -> anyhow::Result<()> {
        tracing::info!(target: "execution", "[DRY-RUN] cancel_all {}", &token_id[..8.min(token_id.len())]);
        Ok(())
    }
}

pub mod risk;

/// Passerelle réelle via le SDK Rust officiel (`polymarket_client_sdk_v2`).
///
/// Chemin d'exécution minimal : client authentifié L1→L2 une fois au
/// démarrage, puis `limit_order()` (le SDK auto-résout tick size, neg_risk
/// et fee rate du marché) → signature EIP-712 locale → POST. Aucune couche
/// intermédiaire.
///
/// Prérequis d'environnement (docs/MVP_REEL.md) :
///   POLYMARKET_PRIVATE_KEY  clé du signer,
///   POLYMARKET_FUNDER       adresse des fonds (selon le type de signature),
///   POLYMARKET_SIG_TYPE     0=EOA (défaut), 1=Proxy, 2=Safe, 3=Poly1271.
#[cfg(feature = "live")]
pub mod live {
    use super::{OrderAck, OrderGateway, OrderRequest, OrderSide, TimeInForce};
    use anyhow::{Context, Result};
    use polymarket_client_sdk_v2::auth::state::Authenticated;
    use polymarket_client_sdk_v2::auth::{LocalSigner, Normal, Signer};
    use polymarket_client_sdk_v2::clob::types::request::CancelMarketOrderRequest;
    use polymarket_client_sdk_v2::clob::types::{OrderType, Side, SignatureType};
    use polymarket_client_sdk_v2::clob::{Client, Config};
    use polymarket_client_sdk_v2::types::{Address, Decimal, U256};
    use polymarket_client_sdk_v2::POLYGON;
    use std::str::FromStr;

    pub struct LiveGateway {
        client: Client<Authenticated<Normal>>,
        /// Clé privée gardée pour reconstruire le signer à chaque ordre
        /// (1-2 ordres/5 min : coût négligeable, type simple).
        pk: String,
    }

    impl LiveGateway {
        /// Authentifie une fois (L1→L2). Échoue vite et clairement si une
        /// variable manque : on ne démarre JAMAIS à moitié configuré.
        pub async fn depuis_env() -> Result<Self> {
            let pk = std::env::var("POLYMARKET_PRIVATE_KEY")
                .context("POLYMARKET_PRIVATE_KEY manquante")?;
            let signer = LocalSigner::from_str(pk.trim())
                .context("clé privée invalide")?
                .with_chain_id(Some(POLYGON));
            let mut builder = Client::new("https://clob.polymarket.com", Config::default())?
                .authentication_builder(&signer);
            if let Ok(funder) = std::env::var("POLYMARKET_FUNDER") {
                let addr: Address = funder.trim().parse().context("POLYMARKET_FUNDER invalide")?;
                builder = builder.funder(addr);
            }
            match std::env::var("POLYMARKET_SIG_TYPE").as_deref() {
                Ok("1") => builder = builder.signature_type(SignatureType::Proxy),
                Ok("2") => builder = builder.signature_type(SignatureType::GnosisSafe),
                Ok("3") => builder = builder.signature_type(SignatureType::Poly1271),
                _ => {} // 0 = EOA (défaut SDK)
            }
            let client = builder.authenticate().await.context("authentification CLOB")?;
            tracing::info!("LiveGateway authentifiée (L1→L2 ok)");
            Ok(Self { client, pk: pk.trim().to_string() })
        }

        /// Préchauffe le cache interne du SDK (tick size, neg_risk, frais)
        /// pour les tokens d'une fenêtre : le premier ordre réel économise
        /// ~300-500 ms d'allers-retours (POST mesuré à 1 188 ms à froid).
        pub async fn prechauffer(&self, tokens: &[String]) {
            for t in tokens {
                if let Ok(id) = t.parse::<U256>() {
                    let _ = self.client.tick_size(id).await;
                    let _ = self.client.neg_risk(id).await;
                    let _ = self.client.fee_rate_bps(id).await;
                }
            }
            tracing::debug!("cache marché préchauffé ({} tokens)", tokens.len());
        }

        /// Nombre d'ordres ouverts sur un token (vérifications du test A-Z).
        pub async fn nb_ordres_ouverts(&self, token_id: &str) -> Result<usize> {
            use polymarket_client_sdk_v2::clob::types::request::OrdersRequest;
            let req = OrdersRequest::builder()
                .asset_id(token_id.parse::<U256>().context("token_id invalide")?)
                .build();
            let page = self.client.orders(&req, None).await.context("liste des ordres")?;
            Ok(page.data.len())
        }

        /// Solde de collatéral (pUSD) du funder — appelé au démarrage :
        /// on n'arme jamais un bot sans savoir ce qu'il a en poche.
        pub async fn solde_collateral(&self) -> Result<f64> {
            use polymarket_client_sdk_v2::clob::types::request::BalanceAllowanceRequest;
            use polymarket_client_sdk_v2::clob::types::AssetType;
            let r = self
                .client
                .balance_allowance(
                    BalanceAllowanceRequest::builder()
                        .asset_type(AssetType::Collateral)
                        .build(),
                )
                .await
                .context("lecture du solde")?;
            let b: f64 = r.balance.to_string().parse().unwrap_or(0.0);
            Ok(b / 1e6) // USDC 6 décimales
        }

        fn tif(t: TimeInForce) -> OrderType {
            match t {
                TimeInForce::Gtc => OrderType::GTC,
                TimeInForce::Fok => OrderType::FOK,
                TimeInForce::Fak => OrderType::FAK,
            }
        }
    }

    impl OrderGateway for LiveGateway {
        async fn post_order(&self, req: OrderRequest) -> Result<OrderAck> {
            let token: U256 = req.token_id.parse().context("token_id invalide")?;
            // Prix au tick (0,001 max supporté), taille bornée à 2 décimales.
            // normalize() retire les zéros de fin : 0.010 → 0.01 — le CLOB
            // refuse plus de décimales que le tick du marché.
            let price = Decimal::from_str(&format!("{:.3}", req.price))?.normalize();
            let size = Decimal::from_str(&format!("{:.2}", req.size))?.normalize();
            let side = match req.side {
                OrderSide::Buy => Side::Buy,
                OrderSide::Sell => Side::Sell,
            };
            // Le builder du SDK récupère tick size / neg_risk / fee rate du
            // marché et refuse les valeurs hors tick : dernière validation
            // avant signature.
            let signer = LocalSigner::from_str(&self.pk)?.with_chain_id(Some(POLYGON));
            // Le builder du SDK auto-résout tick size / neg_risk / fee rate
            // et refuse les valeurs hors tick : dernière validation avant
            // signature EIP-712 locale puis POST.
            let resp = self
                .client
                .limit_order()
                .token_id(token)
                .price(price)
                .size(size)
                .side(side)
                .order_type(Self::tif(req.tif))
                .build_sign_and_post(&signer)
                .await
                .context("build/sign/post ordre")?;
            let taking: f64 = resp.taking_amount.to_string().parse().unwrap_or(0.0);
            let making: f64 = resp.making_amount.to_string().parse().unwrap_or(0.0);
            // BUY taker : making = USDC engagés, taking = parts reçues.
            let (parts, prix) = if taking > 0.0 {
                (taking, Some(making / taking))
            } else {
                (0.0, None)
            };
            tracing::info!(
                target: "execution",
                "[LIVE] ordre {} → {:?} : {:.2} parts exécutées @ {} ({:?} demandé {} x {:.2} @ {:.3}, {})",
                resp.order_id, resp.status, parts,
                prix.map(|p| format!("{p:.4}")).unwrap_or_else(|| "—".into()),
                req.side, &req.token_id[..8.min(req.token_id.len())], req.size, req.price, req.tag
            );
            Ok(OrderAck {
                order_id: resp.order_id.to_string(),
                accepted: true,
                detail: format!("{:?}", resp.status),
                taille_executee: parts,
                prix_reel: prix,
            })
        }

        async fn cancel_all(&self, token_id: &str) -> Result<()> {
            let req = CancelMarketOrderRequest::builder()
                .asset_id(token_id.parse::<U256>().context("token_id invalide")?)
                .build();
            self.client
                .cancel_market_orders(&req)
                .await
                .context("annulation des ordres du marché")?;
            tracing::info!(target: "execution", "[LIVE] cancel_all {}", &token_id[..8.min(token_id.len())]);
            Ok(())
        }
    }
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
