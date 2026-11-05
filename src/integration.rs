//! Transport-neutral contracts for future in-process integrations.
//!
//! Events are bounded notifications that state changed. They are not a durable
//! log; consumers must refresh authoritative state after subscribing or lagging.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use serde::Serialize;
use tokio::sync::broadcast;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DeviceCapabilities {
    pub open: bool,
    pub close: bool,
    pub stop: bool,
    pub set_position: bool,
    pub position_feedback: bool,
}

impl DeviceCapabilities {
    /// Current production controller supports fixed end-stop actions and reports
    /// position. Arbitrary percentage positioning is not exposed.
    pub const WAZOMBI: Self = Self {
        open: true,
        close: true,
        stop: true,
        set_position: false,
        position_feedback: true,
    };

    /// Simulators support the same command and observation contract as hardware.
    pub const SIMULATOR: Self = Self::WAZOMBI;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateChange {
    /// Monotonically increasing within this process. Use a fresh snapshot after
    /// a restart or a broadcast lag error.
    pub revision: u64,
}

#[derive(Clone)]
pub struct StateChangePublisher {
    sender: broadcast::Sender<StateChange>,
    revision: Arc<AtomicU64>,
}

impl Default for StateChangePublisher {
    fn default() -> Self {
        let (sender, _) = broadcast::channel(128);
        Self {
            sender,
            revision: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl StateChangePublisher {
    pub fn subscribe(&self) -> broadcast::Receiver<StateChange> {
        self.sender.subscribe()
    }

    /// Nonblocking publication: a slow subscriber may lag and must resync.
    pub fn publish(&self) {
        let revision = self.revision.fetch_add(1, Ordering::Relaxed) + 1;
        let _ = self.sender.send(StateChange { revision });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn subscribers_receive_revisions_and_lag_is_reported() {
        let publisher = StateChangePublisher::default();
        let mut first = publisher.subscribe();
        let mut second = publisher.subscribe();
        publisher.publish();
        assert_eq!(first.recv().await.unwrap().revision, 1);
        assert_eq!(second.recv().await.unwrap().revision, 1);

        for _ in 0..140 {
            publisher.publish();
        }
        assert!(matches!(
            first.try_recv(),
            Err(broadcast::error::TryRecvError::Lagged(_))
        ));
    }
}
