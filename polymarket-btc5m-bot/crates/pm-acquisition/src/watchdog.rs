//! Watchdog de fraîcheur des flux : publie `FeedStale` sur le bus quand un
//! flux devient silencieux. Les stratégies DOIVENT couper toute prise de
//! risque sur signal stale (règle « pas le droit à l'erreur »).

use crate::{now_ms, Bus};
use pm_core::BusEvent;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time;

#[derive(Debug, Clone)]
pub struct Watchdog {
    last_seen: Arc<Mutex<HashMap<String, u64>>>,
    threshold_ms: u64,
}

impl Watchdog {
    pub fn new(threshold_ms: u64) -> Self {
        Self {
            last_seen: Arc::new(Mutex::new(HashMap::new())),
            threshold_ms,
        }
    }

    /// À appeler à chaque donnée reçue sur `stream`.
    pub fn touch(&self, stream: &str) {
        self.last_seen
            .lock()
            .unwrap()
            .insert(stream.to_string(), now_ms());
    }

    /// Silence courant (ms) d'un flux, None si jamais vu.
    pub fn silent_ms(&self, stream: &str) -> Option<u64> {
        let m = self.last_seen.lock().unwrap();
        m.get(stream).map(|t| now_ms().saturating_sub(*t))
    }

    pub fn is_stale(&self, stream: &str) -> bool {
        self.silent_ms(stream).is_none_or(|s| s > self.threshold_ms)
    }

    /// Boucle de surveillance : émet FeedStale à chaque franchissement.
    pub async fn run(self, bus: Bus) {
        let mut interval = time::interval(Duration::from_millis(500));
        let mut already_stale: HashMap<String, bool> = HashMap::new();
        loop {
            interval.tick().await;
            let snapshot: Vec<(String, u64)> = {
                let m = self.last_seen.lock().unwrap();
                m.iter().map(|(k, v)| (k.clone(), *v)).collect()
            };
            let now = now_ms();
            for (stream, last) in snapshot {
                let silent = now.saturating_sub(last);
                let stale = silent > self.threshold_ms;
                let was = already_stale.get(&stream).copied().unwrap_or(false);
                if stale && !was {
                    tracing::warn!("flux {stream} stale ({silent} ms de silence)");
                    bus.publish(BusEvent::FeedStale {
                        stream: stream.clone(),
                        silent_ms: silent,
                    });
                }
                already_stale.insert(stream, stale);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_logic() {
        let w = Watchdog::new(1_000);
        assert!(w.is_stale("rtds"), "flux jamais vu = stale");
        w.touch("rtds");
        assert!(!w.is_stale("rtds"));
        assert!(w.silent_ms("rtds").unwrap() < 100);
    }
}
