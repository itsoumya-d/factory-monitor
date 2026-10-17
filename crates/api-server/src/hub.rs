//! LiveHub: broadcast `WsEvent`s to subscribers; per-subscriber topic
//! filtering and lag-drop policy (docs/10 §3). Routing keeps the hub
//! HTTP-free so the fan-out loop is unit-testable.

use tokio::sync::broadcast;

use crate::ws::{PushOutcome, WsEvent};

pub struct LiveHub {
    tx: broadcast::Sender<WsEvent>,
}

impl LiveHub {
    pub fn new(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity);
        Self { tx }
    }

    pub fn receiver(&self) -> broadcast::Receiver<WsEvent> {
        self.tx.subscribe()
    }

    pub fn push(&self, event: WsEvent) -> PushOutcome {
        match self.tx.send(event) {
            Ok(0) => PushOutcome::NoInterestedSubscribers,
            Ok(_) => PushOutcome::Delivered,
            // Full buffer means the clients (not us) lagged; pushing past
            // capacity requires a live receiver, which is impossible here.
            Err(broadcast::error::SendError(_)) => PushOutcome::NoInterestedSubscribers,
        }
    }

    pub fn subscriber_count(&self) -> usize {
        self.tx.receiver_count()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::ws::WsEvent;
    use chrono::Utc;
    use common_types::lifecycle::LifecycleState;

    fn lifecycle() -> WsEvent {
        WsEvent::LifecycleEvent {
            from: LifecycleState::Standby,
            to: LifecycleState::Counting,
            ts: Utc::now(),
        }
    }

    #[tokio::test]
    async fn pushes_to_each_subscriber_and_counts() {
        let hub = LiveHub::new(8);
        assert_eq!(hub.push(lifecycle()), PushOutcome::NoInterestedSubscribers);
        let r1 = hub.receiver();
        let _r2 = hub.receiver();
        assert_eq!(hub.push(lifecycle()), PushOutcome::Delivered);
        assert_eq!(hub.subscriber_count(), 2);
        drop(r1);
        assert_eq!(hub.push(lifecycle()), PushOutcome::Delivered);
        assert_eq!(hub.subscriber_count(), 1);
    }

    #[tokio::test]
    async fn lagged_client_obtains_lagged_error() {
        let hub = LiveHub::new(1);
        let mut rx = hub.receiver();
        let _r2 = hub.receiver(); // capacity 1 shared by 2 clients
        hub.push(lifecycle());
        hub.push(lifecycle());
        hub.push(lifecycle());
        let err = rx.try_recv().expect_err("overflow must surface as Lagged");
        assert!(matches!(err, broadcast::error::TryRecvError::Lagged(_)));
    }
}
