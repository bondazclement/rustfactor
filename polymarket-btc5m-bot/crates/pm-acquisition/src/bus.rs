//! Bus interne : diffusion des événements normalisés vers les consommateurs
//! (stratégies, monitoring). Broadcast non bloquant — un consommateur lent
//! perd des messages (lag) plutôt que de ralentir l'acquisition, et le lag
//! est observable côté consommateur (`RecvError::Lagged`).

use pm_core::BusEvent;
use tokio::sync::broadcast;

#[derive(Debug, Clone)]
pub struct Bus {
    tx: broadcast::Sender<BusEvent>,
}

impl Bus {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    pub fn publish(&self, ev: BusEvent) {
        // Erreur uniquement si aucun abonné : sans importance.
        let _ = self.tx.send(ev);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<BusEvent> {
        self.tx.subscribe()
    }
}

impl Default for Bus {
    fn default() -> Self {
        Self::new(65_536)
    }
}
