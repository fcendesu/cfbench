# ADR-0001: Use reqwest as the HTTP client

- Status: Accepted
- Date: 2026-07-19

## Context

`cfbench` needs an asynchronous HTTP client capable of:

- TLS;
- connection reuse;
- streaming response bodies;
- streaming or reusable upload bodies;
- request and response headers;
- HTTP/1.1 and HTTP/2;
- cancellation through dropped futures and Tokio tasks;
- practical cross-platform support;
- IPv4/IPv6 control or a path to a custom connector.

Tower was considered, but Tower is primarily a generic `Service` and middleware abstraction rather than a complete HTTP client. Middleware such as retries, buffering, or rate limiting could also change measurement timing.

Hyper provides lower-level control but would require more connector, body, TLS, and ergonomics work before the first useful measurement can be implemented.

## Decision

Use asynchronous `reqwest::Client` on Tokio for MVP `0.1.0`.

Do not place Tower middleware in the timing path.

Use explicit measurement orchestration for:

- timeouts;
- cancellation;
- errors;
- concurrency;
- request scheduling;
- loaded-latency probes.

Use rustls rather than platform-native TLS unless a supported-platform problem requires revisiting the decision.

## Consequences

### Positive

- Small implementation surface for reliable HTTP requests.
- Straightforward streaming with response byte streams.
- Shared connection pool and TLS configuration.
- Mature ecosystem and broad platform support.
- Easy integration with Tokio, Clap, and Serde.
- Production code remains focused on measurement rules rather than HTTP plumbing.

### Negative

- Native timing cannot reproduce browser `PerformanceResourceTiming` exactly.
- Exact on-wire header-byte accounting is not exposed consistently.
- Forcing address family may require a resolver or connector beyond the simplest builder configuration.
- HTTP/3 support is an unstable opt-in in current reqwest documentation and is not an MVP requirement.
- If future parity requires connection-phase timing hooks, reqwest may become insufficient.

## Guardrails

- No automatic retries.
- No response decompression for measurement traffic.
- No body buffering proportional to download size.
- One client per run configuration.
- Explicit per-request timeout and cancellation.
- Record negotiated HTTP version where available.
- Keep measurement and statistics APIs independent of reqwest-specific types so the transport can be replaced.

## Reconsideration triggers

Re-evaluate reqwest if any of these become mandatory:

- stable HTTP/3 parity;
- exact DNS, connect, TLS, request-start, first-byte, and wire-byte instrumentation;
- a connector that reqwest cannot expose;
- evidence that reqwest overhead limits measurement on target hardware;
- packet-level accounting required by the product.

The likely replacement path would be Hyper/Hyper-Util or a dedicated transport abstraction, not Tower alone.

## References

- reqwest: <https://docs.rs/reqwest/latest/reqwest/>
- Hyper client documentation: <https://docs.rs/hyper/latest/hyper/client/>
- Tower: <https://docs.rs/tower/latest/tower/>
