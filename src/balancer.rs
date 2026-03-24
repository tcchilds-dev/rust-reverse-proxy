use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

pub mod two_random_choices;

pub trait LoadBalancer: Send + Sync {
    fn pick(&self) -> Option<BackendGuard>;
}

pub struct BackendGuard {
    pub url: String,
    counter: Arc<AtomicUsize>,
}

impl BackendGuard {
    pub fn new(url: String, counter: Arc<AtomicUsize>) -> Self {
        counter.fetch_add(1, Ordering::Relaxed);
        Self { url, counter }
    }
}

impl Drop for BackendGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::Relaxed);
    }
}
