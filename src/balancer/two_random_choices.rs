use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use rand::RngExt;

use crate::balancer::{BackendGuard, LoadBalancer};

struct Backend {
    url: String,
    active_connections: Arc<AtomicUsize>,
}

pub struct TwoRandomChoicesBalancer {
    backends: Vec<Backend>,
}

impl TwoRandomChoicesBalancer {
    pub fn new(urls: Vec<String>) -> Self {
        let backends = urls
            .into_iter()
            .map(|url| Backend {
                url,
                active_connections: Arc::new(AtomicUsize::new(0)),
            })
            .collect();

        Self { backends }
    }

    pub fn guard(&self, idx: usize) -> BackendGuard {
        let backend = &self.backends[idx];
        BackendGuard::new(backend.url.clone(), Arc::clone(&backend.active_connections))
    }
}

impl LoadBalancer for TwoRandomChoicesBalancer {
    fn pick(&self) -> BackendGuard {
        let len = self.backends.len();

        match len {
            1 => self.guard(0),
            2 => {
                let a = self.backends[0].active_connections.load(Ordering::Relaxed);
                let b = self.backends[1].active_connections.load(Ordering::Relaxed);
                if a <= b { self.guard(0) } else { self.guard(1) }
            }
            _ => {
                let mut rng = rand::rng();
                let i = rng.random_range(0..len);
                let mut j = rng.random_range(0..len - 1);
                if i <= j {
                    j += 1
                };

                let a = self.backends[i].active_connections.load(Ordering::Relaxed);
                let b = self.backends[j].active_connections.load(Ordering::Relaxed);
                if a <= b { self.guard(j) } else { self.guard(i) }
            }
        }
    }
}
