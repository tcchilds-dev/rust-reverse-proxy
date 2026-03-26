mod common;

use std::time::{Duration, Instant};

use common::{spawn_proxy_with_concurrency_limit, spawn_slow_backend};
use proxy::config::RouteConfig;
use reqwest::StatusCode;

#[tokio::test]
async fn concurrency_limit_queues_excess_requests() {
    // Backend takes 1s per request.
    let slow_port = spawn_slow_backend(Duration::from_secs(1)).await;

    // Only allow 1 concurrent request through the proxy.
    let proxy_port = spawn_proxy_with_concurrency_limit(
        vec![RouteConfig {
            path_prefix: "/".to_string(),
            backends: vec![format!("http://127.0.0.1:{slow_port}")],
        }],
        1,
    )
    .await;

    let client = reqwest::Client::new();
    let start = Instant::now();

    // Fire 3 requests concurrently. With concurrency limit of 1, they should
    // be serialized (queued), taking ~3s total instead of ~1s.
    let mut handles = Vec::new();
    for i in 0..3 {
        let client = client.clone();
        handles.push(tokio::spawn(async move {
            let url = format!("http://127.0.0.1:{proxy_port}/test/{i}");
            let resp = client.get(&url).send().await.unwrap();
            resp.status()
        }));
    }

    let mut statuses = Vec::new();
    for handle in handles {
        statuses.push(handle.await.unwrap());
    }

    let elapsed = start.elapsed();

    // All requests should succeed (ConcurrencyLimitLayer queues, doesn't reject).
    assert!(
        statuses.iter().all(|s| *s == StatusCode::OK),
        "all requests should succeed, got: {statuses:?}"
    );

    // With limit=1 and 3 requests to a 1s backend, total time should be >= 2.5s
    // (some slack for connection overhead).
    assert!(
        elapsed >= Duration::from_millis(2500),
        "expected serialized execution (~3s), but completed in {elapsed:?}"
    );
}
