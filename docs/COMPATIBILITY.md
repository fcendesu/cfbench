# Cloudflare Speedtest compatibility

cfbench 0.3.2 is based on [Cloudflare Speedtest v1.13.0](https://github.com/cloudflare/speedtest/releases/tag/v1.13.0), commit [`5954dee4cc83548a9e5031140df4548f71cd1458`](https://github.com/cloudflare/speedtest/commit/5954dee4cc83548a9e5031140df4548f71cd1458).

Cloudflare Speedtest v1.13.0 leaves the v1.12.1 measurement methodology unchanged. cfbench matches the public default order of latency, download, upload, and loaded-latency work, including the two-request latency phases interleaved between transfer groups. The default run accumulates all 42 successful idle-latency probes in measurement order. It uses the upstream median latency, 90th-percentile bandwidth, 10 ms bandwidth-eligibility, 250 ms loaded-latency, 400 ms probe-throttle, latest-20 loaded points, and strict greater-than-1000 ms adaptive-stop rules.

For `Server-Timing`, cfbench gives the `cfReqDur` and `cfRequestDuration` name variants priority, otherwise sums matching `cfSpeed*` phases, and ignores totals at or below 0.01 ms. Download duration uses the native total request duration minus reported Cloudflare server time. Upload duration uses the native time to response headers, corresponding as closely as reqwest exposes it to the upstream TTFB formula.

The tool streams response bodies and upload data, disables automatic response decompression and automatic retries, reuses one client per run, and records negotiated HTTP version where reqwest exposes it.

## Optional upstream authorization

v1.13.0 adds the optional `authorizationToken` configuration for attributing a test to a registered customer. Upstream sends it as the URL-encoded `jwt` query parameter only to selected HTTPS measurement, TURN-credential, and results-logging endpoints. It defaults to `null`, so ordinary unattributed tests are unchanged.

cfbench does not expose or send this token. Its public Cloudflare download and upload tests do not require one, and cfbench does not implement upstream TURN credentials or results logging. Metadata and the optional RPKI diagnostic are excluded from the upstream authorization mechanism.

## Intentional native differences

Cloudflare's browser engine uses `PerformanceResourceTiming`. A native reqwest client cannot reproduce the browser's connection-phase timing boundaries, `PerformanceResourceTiming.transferSize`, exact on-wire header accounting, browser cache/service-worker behavior, or v1.13.0's HTTP/1.1 server-time-delta calibration. cfbench therefore reports actual payload bytes with Cloudflare's 0.5% overhead estimate and native request/stream durations; it does not claim identical results with the website.

When `Server-Timing` has no usable Cloudflare duration, cfbench follows v1.13.0 and uses a 0 ms fallback. Packet loss and AIM scores are not implemented: packet loss is not shown or approximated with ICMP.

## Automation and diagnostics

On an interactive terminal, ordinary text mode shows one transient, compact
progress line and clears it before the final summary. It presents provisional
rolling throughput and current-request percentage during transfers, running
latency/jitter during latency work, and the latest direction-local loaded
latency when available. These displays are not final measurements: final
p90/median reductions remain authoritative. Upload live rate is
transport-consumption feedback from the request body stream, rather than a
completed upload bandwidth result.

This is presentation-only measurement isolation: telemetry neither changes
measurement timing nor schema-v1 output. `--verbose` retains permanent
per-request lines. For redirected default output and in `--json` and `--quiet`
modes, dynamic progress is not emitted and behavior is unchanged. `--json`
emits one schema-v1 document on stdout, while `--quiet` emits no normal output
and reports the outcome through its exit status; a complete quiet run is fully
silent, while partial and failed runs may print only their terminal error to
stderr. `--quiet` cannot be combined with `--json` or `--verbose`.

Exit status `0` means complete, `1` means failure, cancellation, or no usable
measurement, `2` means invalid command-line usage, and `3` means a usable
partial measurement. JSON remains schema-v1 and measurement behavior is
unchanged.

`--rpki-check` performs a post-plan request to Cloudflare's intentionally RPKI-invalid hostname with a fixed five-second deadline. It reuses the configured no-retry client and strict IPv4 or IPv6 selection. The additive schema-v1 `rpki` result is informational: `reachable` means route filtering was not observed on this path, `unreachable` is consistent with filtering but is not proof, and `error` supports no filtering conclusion. The check creates no timing point, adds no payload usage, does not affect reductions, and does not change the measurement exit status.

## Live tests

```bash
# Contacts Cloudflare and consumes network traffic; ignored by default and not run in CI.
cargo test --test live_cloudflare -- --ignored
```
