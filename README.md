# Reverse Proxy

> [!NOTE] This README was (re-)written by myself. In respect of the reader's time.

A reverse proxy written in Rust using [Axum](https://github.com/tokio-rs/axum) and [Tower](https://github.com/tower-rs/tower).

I picked up Rust because I wanted to learn a low-level language, having at that
stage only had decent experience in JavaScript and Python (I had a little early
experience with C from CS50, but that was little more than a fever dream). I
won't gush about it, but I loved Rust, and I found it took away a lot of the
scary parts of other low-level languages (at least for my purposes).

The proxy idea came from my reading of a blog post about a paper on load
balancing algorithms, posted on Hacker News. I'm afraid the blog escapes me, but
the paper was [The Power of Two Random Choices](https://www.eecs.harvard.edu/~michaelm/postscripts/handbook2001.pdf). That got me looking
in to proxies, which seemed like a great way to practice a low-level language.

[Here's a nice little animation showing off two random choices.](https://x.com/GrantSlatton/status/1754912113246798036)

## Features

- **Load balancing**: using the power-of-two-random-choices algorithm.
- **Response caching** — in-memory cache using [Moka](https://docs.rs/moka/latest/moka/) with TTL sourced from
  `Cache-Control: max-age` or a configurable default.
- **Rate limiting** — per-IP token bucket using [governor](https://docs.rs/governor/latest/governor/).
- **Concurrency limiting** — global cap on in-flight requests; excess requests are queued by Tower.
- **TLS Termination** — optional HTTPS listener (Rustls) that runs alongside HTTP via `tokio::select!`.
- **Observability** — Prometheus metrics, structured access logging, and distributed tracing via Tower's `TraceLayer`.
- **Security** — hop-by-hop header stripping, configurable sensitive header suppression, `X-Forwarded-*` injection.
- **Connection pooling** — tunable keep-alive pool for upstream connections.

## Architecture

Requests pass through a Tower middleware stack before reaching the proxy handler:

```
TraceLayer → AccessLogLayer → RateLimitLayer → ConcurrencyLimitLayer → TimeoutLayer → MetricsLayer → proxy_handler()
```

`proxy_handler()` in `src/proxy.rs`:

1. Checks the in-memory response cache (GET/HEAD only; bypassed if an `Authorization` or `Cookie` header is present).
2. Matches the request path against configured route prefixes.
3. Selects a backend via the load balancer.
4. Strips hop-by-hop and configured sensitive headers; injects `X-Forwarded-For/Host/Proto` and `X-Request-ID`.
5. Forwards the request upstream using `reqwest`.
6. Stores cacheable responses in the cache.
7. Streams the response back to the client.

The `/metrics` endpoint is served on a separate router that bypasses the proxy
middleware stack entirely (no rate limiting, no timeouts).

### Load Balancer

`TwoRandomChoicesBalancer` implements the [power-of-two-random-choices](https://www.eecs.harvard.edu/~michaelm/postscripts/mythesis.pdf) algorithm: sample two random healthy backends and route to the one with fewer active connections. This gives near-optimal load distribution at O(1) cost per request without tracking the full backend state.

Connection counts are maintained via a `ConnectionGuard` RAII wrapper — the count increments on creation and decrements on drop, so it stays accurate even if the request panics or is cancelled.

Each backend has an independent health-check loop running as a Tokio task. A circuit breaker requires 3 consecutive failures to mark a backend unhealthy and 3 consecutive successes to restore it. The tasks are aborted when the balancer is dropped.

## Installation

```bash
cargo build --release
cargo run --release
```

The proxy reads `config.toml` from the working directory on startup.
Edit the `[[routes]]` sections to point at your backends before running.

```toml
[[routes]]
path_prefix = "/api"
backends = ["http://localhost:3001", "http://localhost:3002"]
```

A minimal monitoring stack (Prometheus + Grafana) is included:

```bash
cd monitoring && docker-compose up -d
# Prometheus: http://localhost:9090
# Grafana:    http://localhost:3000
```

A simple Grafana dashboard `.json` config is included.

## Configuration Reference

```toml
[server]
addr = "0.0.0.0:8080"
request_timeout_secs = 30       # requests exceeding this get a 504
max_concurrent_requests = 1000  # excess requests are queued
# max_response_body_bytes = 104857600  # 100 MiB; larger responses bypass cache

[tls]                           # omit section to run HTTP-only
cert_path = "/etc/proxy/cert.pem"
key_path  = "/etc/proxy/key.pem"
addr = "0.0.0.0:8443"

[logging]
level = "info"                  # debug | info | warn | error

[rate_limiting]
requests_per_second = 100       # steady-state allowance per client IP
burst_size = 200                # token bucket burst capacity

[health_checks]
path = "/health"                # path polled on each backend
interval_secs = 10
timeout_secs = 3

[caching]
max_capacity = 500              # maximum number of cached entries
default_ttl = 60                # seconds; used when upstream sends no Cache-Control

[connection_pool]
max_idle_per_host = 32          # idle connections kept per upstream host
idle_timeout_secs = 90          # seconds before an idle connection is closed

[sensitive_headers]
# strip_from_request  = ["x-internal-auth"]  # prevent clients from forging trusted headers
# strip_from_response = ["server", "x-powered-by"]  # suppress server fingerprinting

[[routes]]
path_prefix = "/a"
backends = ["http://localhost:3001", "http://localhost:3002"]

[[routes]]
path_prefix = "/b"
backends = ["http://localhost:3005"]
```

> [!WARNING] Routes are matched top-down. Make sure to order routes from most to least specific.

## Testing

Tests use real servers bound to OS-assigned ports so they can run in parallel
without collisions. There are no mocks — each test exercises the full
production middleware stack.

```bash
cargo test                            # all tests
cargo test --test forwarding_test     # single file
cargo test -- --nocapture --test-threads=1  # with log output
```

## AI Usage Report

Intellectual honesty is important to me. Below I'll detail AI involvement for
each feature.

**Entirely** written by me:

- Main server set up
- Config set up and conversion
- `thiserror` ProxyError implementation
- Proxy handler
- Load balancer implementation
- RAII connection guard
- `axum` State set up
- `tower` middleware layers for timeout, concurrency limit, `governor` rate limiting, and tracing
- HTTP + HTTPS listeners (TLS termination)
- Active health checks
- `prometheus + grafana` Metrics
- `moka` Response Caching

Assisted by AI, _thoroughly_ checked and edited by myself:

- Comments
- README (originally AI generated, re-written by me)
- Post review tweaks and changes:
  - bug fixes (some implemented by myself)
  - structured request logging
  - request ID propagation
  - response body size limit for caching
  - sensitive header stripping configuration
  - connection pooling configuration

Largely generated by AI, checked by myself, and edited where necessary:

- Simple test backends for dev purposes
- Integration tests
