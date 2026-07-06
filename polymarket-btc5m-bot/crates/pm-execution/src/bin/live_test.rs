//! pm-live-test — batterie de tests du chemin d'ordres RÉEL, de A à Z.
//!
//! Conçu pour être lancé par `scripts/tester-ordres.sh` (qui demande les
//! credentials et découvre la fenêtre active). Risque maximal : un ordre
//! GTC à 0,01 $ × 5 parts (5 ¢ engagés, ne peut pas être exécuté sur un
//! carnet coté ~0,50), annulé dans la foulée.
//!
//! Sortie : chaque étape avec sa latence mesurée, PASS/FAIL, code retour 0
//! si tout passe.

use anyhow::{ensure, Context, Result};
use pm_execution::live::LiveGateway;
use pm_execution::{OrderGateway, OrderRequest, OrderSide, TimeInForce};
use std::time::Instant;

fn etape(n: u32, titre: &str) {
    println!("\n[{n}/7] {titre}");
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info,pm_execution=info")
        .init();
    println!("══ pm-live-test — chemin d'ordres réel, batterie A→Z ══");
    let token_up = std::env::var("TOKEN_ID").context("TOKEN_ID manquant (le script le fournit)")?;

    etape(1, "Authentification L1→L2 (dérivation des credentials API)");
    let t = Instant::now();
    let gw = LiveGateway::depuis_env().await?;
    let d_auth = t.elapsed();
    println!("  PASS — {} ms", d_auth.as_millis());

    etape(2, "Lecture du solde de collatéral");
    let t = Instant::now();
    let solde = gw.solde_collateral().await?;
    println!("  PASS — {:.2} $ disponibles ({} ms)", solde, t.elapsed().as_millis());
    ensure!(solde >= 0.10, "solde insuffisant pour le test (≥ 0,10 $ requis)");

    etape(3, "Pose d'un ordre GTC hors marché (BUY 5 parts @ 0,01 $)");
    let t = Instant::now();
    let ack = gw
        .post_order(OrderRequest {
            token_id: token_up.clone(),
            side: OrderSide::Buy,
            price: 0.01,
            size: 5.0,
            tif: TimeInForce::Gtc,
            tag: "live-test".into(),
        })
        .await?;
    let d_post = t.elapsed();
    ensure!(ack.accepted, "ordre refusé: {}", ack.detail);
    println!("  PASS — order_id={} statut={} ({} ms)", ack.order_id, ack.detail, d_post.as_millis());

    etape(4, "Vérification : l'ordre est visible dans les ordres ouverts");
    let t = Instant::now();
    let n = gw.nb_ordres_ouverts(&token_up).await?;
    ensure!(n >= 1, "ordre non visible ({n} ouverts)");
    println!("  PASS — {n} ordre(s) ouvert(s) ({} ms)", t.elapsed().as_millis());

    etape(5, "Annulation de tous les ordres du marché");
    let t = Instant::now();
    gw.cancel_all(&token_up).await?;
    let d_cancel = t.elapsed();
    println!("  PASS — {} ms", d_cancel.as_millis());

    etape(6, "Vérification : plus aucun ordre ouvert");
    let t = Instant::now();
    let n = gw.nb_ordres_ouverts(&token_up).await?;
    ensure!(n == 0, "{n} ordre(s) encore ouvert(s) !");
    println!("  PASS — carnet propre ({} ms)", t.elapsed().as_millis());

    etape(7, "Récapitulatif des latences du chemin d'ordres");
    println!("  auth L1→L2 : {:>5} ms (une fois au démarrage)", d_auth.as_millis());
    println!("  POST ordre : {:>5} ms (le chiffre critique)", d_post.as_millis());
    println!("  annulation : {:>5} ms", d_cancel.as_millis());

    println!("\n══ TOUT PASSE — chemin d'ordres réel opérationnel ══");
    Ok(())
}
