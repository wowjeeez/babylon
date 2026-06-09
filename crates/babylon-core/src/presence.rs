use dashmap::DashMap;
use std::time::Instant;

#[derive(Default)]
pub struct Presence {
    map: DashMap<String, (bool, Instant)>,
}

impl Presence {
    pub fn touch(&self, handle: &str) {
        self.map
            .insert(handle.to_string(), (self.live(handle), Instant::now()));
    }

    pub fn set_live(&self, handle: &str, live: bool) {
        let now = Instant::now();
        self.map
            .entry(handle.to_string())
            .and_modify(|v| v.0 = live)
            .or_insert((live, now));
    }

    #[must_use]
    pub fn live(&self, handle: &str) -> bool {
        self.map.get(handle).is_some_and(|v| v.0)
    }

    #[must_use]
    pub fn online(&self, handle: &str, window_secs: u64) -> bool {
        self.map
            .get(handle)
            .is_some_and(|v| v.0 && v.1.elapsed().as_secs() <= window_secs)
    }
}
