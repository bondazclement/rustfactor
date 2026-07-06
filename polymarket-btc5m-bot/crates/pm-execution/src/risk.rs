//! Garde-fous de risque pour l'exécution RÉELLE — la dernière ligne de
//! défense entre la stratégie et l'argent.
//!
//! `RiskGate` enveloppe n'importe quelle passerelle (`OrderGateway`) et
//! REFUSE tout ordre qui viole une règle. Les règles sont des plafonds durs,
//! jamais assouplis à chaud :
//! - armement explicite (double opt-in : compilation `--features live` ET
//!   variable d'environnement `PM_LIVE_ARME=oui`),
//! - notional maximal par ordre (micro-trading : 10 $ par défaut),
//! - nombre maximal d'ordres par session,
//! - perte cumulée maximale par session (signalée par le moteur au fil des
//!   règlements) → arrêt définitif,
//! - kill-switch manuel/automatique (`declencher_arret`) : déclenché par le
//!   moteur sur toute contradiction de résolution (✗) ou incident de flux.
//!
//! Tout refus est journalisé avec sa raison ; un refus n'est JAMAIS silencieux.

use crate::{OrderAck, OrderGateway, OrderRequest};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;

#[derive(Debug, Clone, Copy)]
pub struct RiskConfig {
    /// Notional maximal par ordre (USDC). Micro-trading : 10 $.
    pub max_notional_par_ordre: f64,
    /// Nombre maximal d'ordres acceptés sur la session.
    pub max_ordres_session: u32,
    /// Perte cumulée (USDC) au-delà de laquelle plus AUCUN ordre ne part.
    pub perte_max_session: f64,
    /// Prix limite maximal accepté à l'achat (cohérent avec la stratégie).
    pub prix_max: f64,
}

impl Default for RiskConfig {
    fn default() -> Self {
        Self {
            max_notional_par_ordre: 10.0,
            max_ordres_session: 20,
            perte_max_session: 30.0,
            prix_max: 0.95,
        }
    }
}

/// Enveloppe de risque autour d'une passerelle réelle.
pub struct RiskGate<G> {
    inner: G,
    cfg: RiskConfig,
    arme: bool,
    arret: AtomicBool,
    raison_arret: Mutex<String>,
    ordres_envoyes: AtomicU32,
    perte_cumulee: Mutex<f64>,
}

impl<G> RiskGate<G> {
    /// `arme` doit venir d'un opt-in explicite (env `PM_LIVE_ARME=oui`).
    pub fn new(inner: G, cfg: RiskConfig, arme: bool) -> Self {
        Self {
            inner,
            cfg,
            arme,
            arret: AtomicBool::new(false),
            raison_arret: Mutex::new(String::new()),
            ordres_envoyes: AtomicU32::new(0),
            perte_cumulee: Mutex::new(0.0),
        }
    }

    /// Accès à la passerelle enveloppée (préchauffage, lectures).
    pub fn interieur(&self) -> &G {
        &self.inner
    }

    /// Kill-switch : après cet appel, plus aucun ordre ne partira jamais
    /// (session entière). Déclenché sur ✗ de résolution, incident de flux,
    /// ou manuellement.
    pub fn declencher_arret(&self, raison: &str) {
        self.arret.store(true, Ordering::SeqCst);
        *self.raison_arret.lock().unwrap() = raison.to_string();
        tracing::error!("KILL-SWITCH DÉCLENCHÉ : {raison} — plus aucun ordre ne partira");
    }

    pub fn est_arrete(&self) -> bool {
        self.arret.load(Ordering::SeqCst)
    }

    /// Le moteur signale chaque PnL de règlement ; une perte cumulée
    /// au-delà du plafond déclenche l'arrêt définitif.
    pub fn signaler_pnl(&self, pnl: f64) {
        let mut p = self.perte_cumulee.lock().unwrap();
        *p += pnl;
        if *p < -self.cfg.perte_max_session {
            drop(p);
            self.declencher_arret(&format!(
                "perte de session {:.2} $ > plafond {:.2} $",
                -self.perte_cumulee.lock().unwrap().min(0.0),
                self.cfg.perte_max_session
            ));
        }
    }

    fn verifier(&self, req: &OrderRequest) -> Result<(), String> {
        if !self.arme {
            return Err("passerelle NON ARMÉE (PM_LIVE_ARME=oui absent)".into());
        }
        if self.est_arrete() {
            return Err(format!(
                "kill-switch actif : {}",
                self.raison_arret.lock().unwrap()
            ));
        }
        let notional = req.price * req.size;
        if notional > self.cfg.max_notional_par_ordre + 1e-9 {
            return Err(format!(
                "notional {notional:.2} $ > plafond {:.2} $",
                self.cfg.max_notional_par_ordre
            ));
        }
        if req.price > self.cfg.prix_max {
            return Err(format!("prix {:.3} > plafond {:.3}", req.price, self.cfg.prix_max));
        }
        if req.price <= 0.0 || req.size <= 0.0 || !req.price.is_finite() || !req.size.is_finite() {
            return Err("prix/taille invalides".into());
        }
        let n = self.ordres_envoyes.load(Ordering::SeqCst);
        if n >= self.cfg.max_ordres_session {
            return Err(format!(
                "{n} ordres déjà envoyés ≥ plafond {}",
                self.cfg.max_ordres_session
            ));
        }
        Ok(())
    }
}

impl<G: OrderGateway> OrderGateway for RiskGate<G> {
    async fn post_order(&self, req: OrderRequest) -> anyhow::Result<OrderAck> {
        if let Err(raison) = self.verifier(&req) {
            tracing::warn!(
                "ORDRE REFUSÉ par la couche de risque : {raison} ({:?} {} x {:.0} @ {:.3})",
                req.side, &req.token_id[..8.min(req.token_id.len())], req.size, req.price
            );
            return Ok(OrderAck {
                order_id: String::new(),
                accepted: false,
                detail: format!("refus risque: {raison}"),
                taille_executee: 0.0,
                prix_reel: None,
            });
        }
        self.ordres_envoyes.fetch_add(1, Ordering::SeqCst);
        self.inner.post_order(req).await
    }

    async fn cancel_all(&self, token_id: &str) -> anyhow::Result<()> {
        // Les annulations sont TOUJOURS autorisées (réduisent le risque).
        self.inner.cancel_all(token_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DryRunGateway, TimeInForce};

    fn req(price: f64, size: f64) -> OrderRequest {
        OrderRequest {
            token_id: "1234567890".into(),
            side: crate::OrderSide::Buy,
            price,
            size,
            tif: TimeInForce::Fak,
            tag: "test".into(),
        }
    }

    #[tokio::test]
    async fn non_arme_refuse_tout() {
        let g = RiskGate::new(DryRunGateway::new(), RiskConfig::default(), false);
        let a = g.post_order(req(0.5, 10.0)).await.unwrap();
        assert!(!a.accepted);
        assert!(a.detail.contains("NON ARMÉE"));
    }

    #[tokio::test]
    async fn plafond_notional_refuse() {
        let g = RiskGate::new(DryRunGateway::new(), RiskConfig::default(), true);
        // 0,60 × 20 = 12 $ > 10 $
        let a = g.post_order(req(0.60, 20.0)).await.unwrap();
        assert!(!a.accepted && a.detail.contains("notional"));
        // 0,60 × 15 = 9 $ ≤ 10 $ → accepté
        let a = g.post_order(req(0.60, 15.0)).await.unwrap();
        assert!(a.accepted);
    }

    #[tokio::test]
    async fn kill_switch_definitif() {
        let g = RiskGate::new(DryRunGateway::new(), RiskConfig::default(), true);
        assert!(g.post_order(req(0.5, 10.0)).await.unwrap().accepted);
        g.declencher_arret("contradiction de résolution ✗");
        let a = g.post_order(req(0.5, 10.0)).await.unwrap();
        assert!(!a.accepted && a.detail.contains("kill-switch"));
        // Les annulations restent permises.
        g.cancel_all("1234567890").await.unwrap();
    }

    #[tokio::test]
    async fn perte_max_declenche_arret() {
        let cfg = RiskConfig {
            perte_max_session: 25.0,
            ..Default::default()
        };
        let g = RiskGate::new(DryRunGateway::new(), cfg, true);
        g.signaler_pnl(-10.0);
        assert!(!g.est_arrete());
        g.signaler_pnl(8.0); // gain : cumul −2
        g.signaler_pnl(-24.0); // cumul −26 < −25
        assert!(g.est_arrete());
        assert!(!g.post_order(req(0.5, 10.0)).await.unwrap().accepted);
    }

    #[tokio::test]
    async fn plafond_ordres_session() {
        let cfg = RiskConfig {
            max_ordres_session: 2,
            ..Default::default()
        };
        let g = RiskGate::new(DryRunGateway::new(), cfg, true);
        assert!(g.post_order(req(0.5, 10.0)).await.unwrap().accepted);
        assert!(g.post_order(req(0.5, 10.0)).await.unwrap().accepted);
        let a = g.post_order(req(0.5, 10.0)).await.unwrap();
        assert!(!a.accepted && a.detail.contains("plafond"));
    }

    #[tokio::test]
    async fn prix_extreme_refuse() {
        let g = RiskGate::new(DryRunGateway::new(), RiskConfig::default(), true);
        assert!(!g.post_order(req(0.97, 5.0)).await.unwrap().accepted);
        assert!(!g.post_order(req(f64::NAN, 5.0)).await.unwrap().accepted);
    }
}
