use crate::error::Result;
use crate::presence::Presence;
use crate::store::Store;
use crate::waiters::Waiters;
use dashmap::DashMap;
use std::sync::Arc;

pub const PRESENCE_WINDOW_SECS: u64 = 90;

pub struct Hub {
    pub store: Store,
    pub waiters: Waiters,
    pub presence: Presence,
    pub waits: DashMap<String, u32>,
}

impl Hub {
    pub async fn new(path: &str) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            store: Store::open(path).await?,
            waiters: Waiters::default(),
            presence: Presence::default(),
            waits: DashMap::default(),
        }))
    }

    pub async fn new_in_memory() -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            store: Store::open_in_memory().await?,
            waiters: Waiters::default(),
            presence: Presence::default(),
            waits: DashMap::default(),
        }))
    }

    #[must_use]
    #[allow(clippy::unused_self)]
    pub fn now_ms(&self) -> i64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
            .unwrap_or(0)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn hub_builds_over_store() {
        let hub = Hub::new_in_memory().await.unwrap();
        assert!(hub.now_ms() > 0);
    }
}
