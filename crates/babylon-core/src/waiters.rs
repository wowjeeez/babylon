use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::Notify;

#[derive(Default)]
pub struct Waiters {
    map: DashMap<String, Arc<Notify>>,
}

impl Waiters {
    #[must_use]
    pub fn for_handle(&self, handle: &str) -> Arc<Notify> {
        self.map
            .entry(handle.to_string())
            .or_insert_with(|| Arc::new(Notify::new()))
            .clone()
    }

    pub fn wake(&self, handle: &str) {
        if let Some(n) = self.map.get(handle) {
            n.notify_waiters();
        }
    }

    pub fn release(&self, handle: &str) {
        self.map.remove_if(handle, |_, v| Arc::strong_count(v) == 1);
    }
}
