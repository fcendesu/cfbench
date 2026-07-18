# cfbench documentation

`cfbench` is an unofficial native Rust CLI that measures Internet connection quality against Cloudflare's speed-test endpoints.

This documentation set defines the product, the MVP boundary, the native measurement model, and the validation strategy before implementation begins.

## Documents

- [Agent instructions](AGENTS.md)
- [Product Requirements Document](docs/PRD.md)
- [MVP Scope](docs/MVP.md)
- [Measurement Compatibility Specification](docs/MEASUREMENT_COMPATIBILITY.md)
- [Test Strategy](docs/TEST_STRATEGY.md)
- [Architecture and Design Specification](docs/superpowers/specs/2026-07-19-cfbench-design.md)
- [ADR-0001: Use reqwest as the HTTP client](docs/adr/0001-http-client-reqwest.md)

## Locked decisions

- Language: Rust
- Product name: `cfbench`
- Interface: plain CLI; no TUI
- Runtime: Tokio
- HTTP client: reqwest
- Primary target: Cloudflare's `__down` and `__up` endpoints
- Compatibility goal: reproduce Cloudflare's public measurement sequence, thresholds, reductions, and result semantics as closely as a native client permits
- MVP packet loss: reported as unavailable; TURN-based packet loss is post-MVP
- Output: human-readable text and versioned JSON

## Terminology

The project should use **Cloudflare-compatible methodology** or **faithful native implementation** in documentation. It must not claim bit-for-bit identity with the browser test because Cloudflare's engine uses browser-only `PerformanceResourceTiming` values.

## Source baseline

The initial compatibility baseline is Cloudflare Speedtest `v1.11.0`, released July 1, 2026. Before implementation or release, compare the current upstream default configuration and algorithms against this baseline.
