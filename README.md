# Reverse Proxy

A reverse proxy written in Rust using [Axum](https://github.com/tokio-rs/axum) and [Tower](https://github.com/tower-rs/tower). Demonstrates load balancing, response caching, rate limiting, observability, and security patterns found in production proxies.

## Features

- **Load balancing** — power-of-two-random-choices algorithm with active health checks and a circuit breaker
- **Response caching** — in-memory cache (Moka) with TTL sourced from `Cache-Control: max-age` or a configurable default
- **Rate limiting** — per-IP token bucket via `governor`; returns `429 Too Many Requests` with a `Retry-After` header
- **Concurrency limiting** — global cap on in-flight requests; excess requests are queued by Tower
- **TLS** — optional HTTPS listener (Rustls) that runs alongside HTTP via `tokio::select!`
- **Observability** — Prometheus metrics, structured access logging, and distributed tracing via Tower's `TraceLayer`
- **Security** — hop-by-hop header stripping, configurable sensitive header suppression, `X-Forwarded-*` injection
- **Connection pooling** — tunable keep-alive pool for upstream connections (reqwest)

## Architecture

Requests pass through a Tower middleware stack before reaching the proxy handler:

```
TraceLayer → AccessLogLayer → RateLimitLayer → ConcurrencyLimitLayer → TimeoutLayer → MetricsLayer → proxy_handler()
```

`proxy_handler()` in `src/proxy.rs`:

1. Checks the in-memory response cache (GET/HEAD only; bypassed if `Authorization` header is present)
2. Matches the request path against configured route prefixes
3. Selects a backend via the load balancer
4. Strips hop-by-hop and configured sensitive headers; injects `X-Forwarded-For/Host/Proto` and `X-Request-ID`
5. Forwards the request upstream using `reqwest`
6. Stores cacheable responses in the cache
7. Streams the response back to the client

The `/metrics` endpoint is served on a separate router that bypasses the proxy middleware stack entirely (no rate limiting, no timeouts).

### Load Balancer

`TwoRandomChoicesBalancer` implements the [power-of-two-random-choices](https://www.eecs.harvard.edu/~michaelm/postscripts/mythesis.pdf) algorithm: sample two random healthy backends and route to the one with fewer active connections. This gives near-optimal load distribution at O(1) cost per request without tracking the full backend state.

Connection counts are maintained via a `ConnectionGuard` RAII wrapper — the count increments on creation and decrements on drop, so it stays accurate even if the request panics or is cancelled.

Each backend has an independent health-check loop running as a Tokio task. A circuit breaker requires 3 consecutive failures to mark a backend unhealthy and 3 consecutive successes to restore it. The tasks are aborted when the balancer is dropped.

### Caching

Cache keys are `"METHOD:full-URI"`. Only responses with [RFC 7231 cacheable status codes](https://www.rfc-editor.org/rfc/rfc7231#section-6.1) (200, 203, 204, 206, 300, 301, 404, 405, 410, 414, 501) are stored. TTL is extracted from the upstream `Cache-Control: max-age` value, falling back to `config.toml`'s `default_ttl`. Response bodies larger than `max_response_body_bytes` are returned to the client but not cached.

## Getting Started

```bash
cargo build --release
cargo run --release
```

The proxy reads `config.toml` from the working directory on startup. Edit the `[[routes]]` sections to point at your backends before running.

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

Routes are matched top-down. Make sure to order routes from most to least specific.

## Testing

Tests use real servers bound to OS-assigned ports so they can run in parallel without collisions. There are no mocks — each test exercises the full production middleware stack.

```bash
cargo test                            # all tests
cargo test --test forwarding_test     # single file
cargo test -- --nocapture --test-threads=1  # with log output
```

## Key Dependencies

| Crate                                     | Role                                                             |
| ----------------------------------------- | ---------------------------------------------------------------- |
| `axum`                                    | HTTP server and router                                           |
| `tower` / `tower-http`                    | Middleware stack (rate limiting, concurrency, timeouts, tracing) |
| `reqwest`                                 | Upstream HTTP client with connection pooling                     |
| `moka`                                    | Async-native, thread-safe in-memory cache                        |
| `governor`                                | Token-bucket rate limiter                                        |
| `axum-server`                             | TLS listener via Rustls                                          |
| `metrics` + `metrics-exporter-prometheus` | Prometheus metrics                                               |
| `tokio`                                   | Async runtime                                                    |

## AI Usage Report

Intellectual honesty is important to me. Below I'll detail AI involvement for each feature.

**Entirely** written by me. No `Claude Code` usage at all:

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

Written by `Claude Code`, checked and understood **thoroughly** by me:

- Comments (some edits made by myself)
- README (I made some corrections, as well as this AI report section)
- After review changes:
  - bug fixes (some implemented by myself)
  - structured request logging
  - request ID propagation
  - response body size limit for caching
  - sensitive header stripping configuration
  - connection pooling configuration

Written by `Claude Code`, checked and understood **moderately** by me:

- Simple test backends for dev purposes (had to make an edit)
- Integration tests (I made sure to understand the structure of the tests)
