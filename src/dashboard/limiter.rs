//! express-rate-limit's MemoryStore: a fixed window per client IP that
//! starts with the client's first request.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::util::now_ms;

pub struct Limiter {
    pub window_ms: i64,
    pub limit: u64,
    m_hits: Mutex<HashMap<String, (u64, i64)>>,
}

/// One request's standing: hits so far and when the window resets.
pub struct Hit {
    pub hits: u64,
    pub reset_ms: i64,
    pub limited: bool,
}

impl Limiter {
    pub fn new(p_window_ms: i64, p_limit: u64) -> Limiter {
        Limiter { window_ms: p_window_ms, limit: p_limit, m_hits: Mutex::new(HashMap::new()) }
    }

    /// Counts a request from p_key.
    pub fn hit(&self, p_key: &str) -> Hit {
        let v_now = now_ms();
        let mut m = self.m_hits.lock().unwrap();
        m.retain(|_, (_, r)| *r > v_now);
        let e = m.entry(p_key.to_string()).or_insert((0, v_now + self.window_ms));
        e.0 += 1;
        Hit { hits: e.0, reset_ms: e.1, limited: e.0 > self.limit }
    }

    pub fn remaining(&self, p_hit: &Hit) -> u64 {
        self.limit.saturating_sub(p_hit.hits)
    }
}

/// Seconds until the window resets (rounded up).
pub fn seconds_left(p_reset_ms: i64) -> i64 {
    ((p_reset_ms - now_ms()).max(0) + 999) / 1000
}
