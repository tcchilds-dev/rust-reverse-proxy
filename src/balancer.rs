//! Load balancer abstraction and connection tracking.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use metrics::gauge;

pub mod two_random_choices;

pub trait LoadBalancer: Send + Sync {
    /// Selects a healthy backend and returns a guard that tracks the active connection.
    /// Returns `None` if no healthy backends are available.
    fn pick(&self) -> Option<ConnectionGuard>;
}

/// RAII guard that increments the backend's active connection count on creation
/// and decrements it on drop. This lets the load balancer make decisions based
/// on real-time connection counts without manual bookkeeping.
pub struct ConnectionGuard {
    pub url: String,
    counter: Arc<AtomicUsize>,
}

impl ConnectionGuard {
    pub fn new(url: String, counter: Arc<AtomicUsize>) -> Self {
        // Relaxed ordering is sufficient here: we only need an approximate count
        // for load-balancing decisions, not strict cross-thread sequencing.
        counter.fetch_add(1, Ordering::Relaxed);
        gauge!("proxy_upstream_active_connections", "upstream" => url.clone()).increment(1.0);
        Self { url, counter }
    }
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::Relaxed);
        gauge!("proxy_upstream_active_connections", "upstream" => self.url.clone()).decrement(1.0);
    }
}
