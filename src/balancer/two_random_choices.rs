//! Power-of-two-random-choices load balancer with active health checking.
//!
//! Instead of tracking every backend's load (expensive under high concurrency),
//! this algorithm samples two random healthy backends and picks the one with
//! fewer active connections. This gives near-optimal load distribution with O(1)
//! overhead per request. See "The Power of Two Choices in Randomized Load
//! Balancing" (Mitzenmacher, 2001).

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use rand::RngExt;

use crate::{
    balancer::{ConnectionGuard, LoadBalancer},
    config::HealthCheckConfig,
};

struct Backend {
    url: String,
    active_connections: Arc<AtomicUsize>,
    is_healthy: Arc<AtomicBool>,
    /// Circuit breaker: backend is marked unhealthy after 3 consecutive failures.
    consecutive_failures: Arc<AtomicUsize>,
    /// Circuit breaker: backend is marked healthy again after 3 consecutive successes.
    consecutive_successes: Arc<AtomicUsize>,
}

/// Load balancer using the power-of-two-random-choices algorithm.
///
/// Spawns a background health check task per backend on construction.
/// Tasks are aborted when the balancer is dropped.
pub struct TwoRandomChoicesBalancer {
    backends: Vec<Arc<Backend>>,
    health_check_tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl Drop for TwoRandomChoicesBalancer {
    fn drop(&mut self) {
        for handle in &self.health_check_tasks {
            handle.abort();
        }
    }
}

impl TwoRandomChoicesBalancer {
    pub fn new(urls: Vec<String>, health_check_config: &HealthCheckConfig) -> Self {
        let backends: Vec<Arc<Backend>> = urls
            .into_iter()
            .map(|url| {
                Arc::new(Backend {
                    url,
                    active_connections: Arc::new(AtomicUsize::new(0)),
                    is_healthy: Arc::new(AtomicBool::new(true)),
                    consecutive_failures: Arc::new(AtomicUsize::new(0)),
                    consecutive_successes: Arc::new(AtomicUsize::new(0)),
                })
            })
            .collect();

        let client = reqwest::Client::new();

        // Spawn one health check loop per backend. Each runs independently and
        // flips the is_healthy flag based on a simple circuit-breaker threshold.
        let health_check_tasks = backends
            .iter()
            .map(|backend| {
                let client = client.clone();
                let backend = Arc::clone(backend);
                let interval = Duration::from_secs(health_check_config.interval_secs);
                let timeout = Duration::from_secs(health_check_config.timeout_secs);
                let path = health_check_config.path.clone();

                tokio::spawn(async move {
                    let mut ticker = tokio::time::interval(interval);

                    loop {
                        ticker.tick().await;
                        let url = format!("{}{}", backend.url, path);

                        let healthy = client
                            .get(&url)
                            .timeout(timeout)
                            .send()
                            .await
                            .map(|r| r.status().is_success())
                            .unwrap_or(false);

                        if healthy {
                            backend.consecutive_failures.store(0, Ordering::Relaxed);
                            let success = backend
                                .consecutive_successes
                                .fetch_add(1, Ordering::Relaxed)
                                + 1;
                            if success >= 3 {
                                backend.is_healthy.store(true, Ordering::Relaxed);
                            }
                        } else {
                            backend.consecutive_successes.store(0, Ordering::Relaxed);
                            let f =
                                backend.consecutive_failures.fetch_add(1, Ordering::Relaxed) + 1;
                            if f >= 3 {
                                backend.is_healthy.store(false, Ordering::Relaxed);
                            }
                        }
                    }
                })
            })
            .collect();

        Self {
            backends,
            health_check_tasks,
        }
    }

    pub fn guard(&self, idx: usize) -> ConnectionGuard {
        let backend = &self.backends[idx];
        ConnectionGuard::new(backend.url.clone(), Arc::clone(&backend.active_connections))
    }
}

impl LoadBalancer for TwoRandomChoicesBalancer {
    fn pick(&self) -> Option<ConnectionGuard> {
        let healthy: Vec<usize> = self
            .backends
            .iter()
            .enumerate()
            .filter(|(_, b)| b.is_healthy.load(Ordering::Relaxed))
            .map(|(i, _)| i)
            .collect();

        match healthy.len() {
            0 => None,
            1 => Some(self.guard(healthy[0])),
            // With exactly 2 backends, no randomness needed — just compare.
            2 => {
                let a = self.backends[healthy[0]]
                    .active_connections
                    .load(Ordering::Relaxed);
                let b = self.backends[healthy[1]]
                    .active_connections
                    .load(Ordering::Relaxed);
                if a <= b {
                    Some(self.guard(healthy[0]))
                } else {
                    Some(self.guard(healthy[1]))
                }
            }
            _ => {
                let mut rng = rand::rng();
                let n = healthy.len();
                // Pick two distinct random indices into the healthy list.
                let i = rng.random_range(0..n);
                let mut j = rng.random_range(0..n - 1);
                if i <= j {
                    j += 1
                };

                let a = self.backends[healthy[i]]
                    .active_connections
                    .load(Ordering::Relaxed);
                let b = self.backends[healthy[j]]
                    .active_connections
                    .load(Ordering::Relaxed);
                // Pick the backend with fewer active connections.
                if a <= b {
                    Some(self.guard(healthy[i]))
                } else {
                    Some(self.guard(healthy[j]))
                }
            }
        }
    }
}
