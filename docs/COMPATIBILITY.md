# Cloudflare Speedtest compatibility

cfbench 0.2.0 is based on [Cloudflare Speedtest v1.12.1](https://github.com/cloudflare/speedtest/releases/tag/v1.12.1), commit [`567aeade7b6e1fbeea98edddb6031c5877678866`](https://github.com/cloudflare/speedtest/commit/567aeade7b6e1fbeea98edddb6031c5877678866).

It matches the public default order of latency, download, upload, and loaded-latency work, including the v1.12.1 two-request latency phases interleaved between transfer groups. It uses the upstream median latency, 90th-percentile bandwidth, 10 ms bandwidth-eligibility, 250 ms loaded-latency, 400 ms probe-throttle, and 1000 ms adaptive-stop rules.

The tool streams response bodies and upload data, disables automatic response decompression and retries, reuses one client per run, and records negotiated HTTP version where reqwest exposes it.

## Intentional native differences

Cloudflare's browser engine uses `PerformanceResourceTiming`. A native reqwest client cannot reproduce browser resource timing, exact on-wire header accounting, browser cache/service-worker behavior, or v1.12.1's TCP connection/server-time-delta calibration. cfbench therefore reports actual payload bytes and native request/stream durations; it does not claim numerical identity with the website.

When `Server-Timing` has no usable Cloudflare duration, cfbench follows v1.12.1 and uses a 0 ms fallback. Packet loss and AIM scores are not implemented: packet loss is not shown or approximated with ICMP.

## Automation and diagnostics

Ordinary text mode prints the final summary without per-request progress. `--verbose` enables live line-oriented progress on stderr. `--quiet` suppresses progress, and `--json` emits exactly one schema-v1 JSON document on stdout without progress.

For automation, exit status `0` means the measurement completed, `2` means a partial result retained at least one accepted latency, download, or upload point, and `1` means no usable measurement was accepted or the run was cancelled. Output-writing failures also exit `1`; invalid command-line input is handled by Clap and exits `2`.

`--rpki-check` performs a post-plan request to Cloudflare's intentionally RPKI-invalid hostname with a fixed five-second deadline. It reuses the configured no-retry client and strict IPv4 or IPv6 selection. The additive schema-v1 `rpki` result is informational: `reachable` means route filtering was not observed on this path, `unreachable` is consistent with filtering but is not proof, and `error` supports no filtering conclusion. The check creates no timing point, adds no payload usage, does not affect reductions, and does not change the measurement exit status.

## Live tests

```bash
# Contacts Cloudflare and consumes network traffic; ignored by default and not run in CI.
cargo test --test live_cloudflare -- --ignored
```
