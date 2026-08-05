# AGENTS.md

## Project

`cfbench` is an unofficial native Rust CLI for measuring connectivity against Cloudflare's speed-test endpoints. It is a plain line-oriented command; do not add a TUI, browser runtime, hosted service, or unrelated provider abstraction.

## Core constraints

- Rust stable, Tokio, reqwest with rustls, Clap, Serde, and `thiserror` are the established stack.
- Reuse one configured HTTP client per run; disable retries and response decompression for measurement traffic.
- Stream downloads and uploads. Do not retain bodies proportional to payload size.
- Keep network I/O, orchestration, statistics, result models, and rendering separate.
- `--json` emits exactly one JSON document to stdout. Progress and diagnostics use stderr.
- `--ipv4` and `--ipv6` are strict and mutually exclusive.
- Packet loss is not measured or displayed. Do not substitute ICMP loss.
- Do not add HTTP/3, TURN/WebRTC, AIM scores, a dashboard, a daemon, or a TUI without an explicit scope decision.

## Compatibility

The public baseline is Cloudflare Speedtest `v1.13.0`, commit `5954dee4cc83548a9e5031140df4548f71cd1458`. Read [docs/COMPATIBILITY.md](docs/COMPATIBILITY.md) before changing schedules, timing, thresholds, or reductions.

Never claim numeric identity with the browser test. Native reqwest timing has different observable boundaries from browser `PerformanceResourceTiming`.

## Privacy and examples

Do not place a user's public IP, ISP, ASN, city, edge location, or copied live results in source, fixtures, tests, or documentation. Use RFC-reserved addresses such as `192.0.2.1` or `2001:db8::1`, ASN `64496`, and clearly synthetic names such as `Example Network`.

## Validation

Run these before committing a code or release change:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
cargo build --release
```

Live tests are ignored by default and must not be required by CI.

## Release process

- Record user-visible changes in [CHANGELOG.md](CHANGELOG.md).
- Releases are created by pushing a `v*` tag; do not publish from ordinary branch pushes.
- The release workflow builds Linux x86_64 `.deb`, `.rpm`, and standalone artifacts with SHA-256 checksums.
- `scripts/install.sh` must keep checksum verification before installation.
- Dependabot updates Cargo and GitHub Actions dependencies weekly.
