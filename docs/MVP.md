# cfbench MVP Scope

- Target version: `0.1.0`
- Date: 2026-07-19

## MVP objective

Deliver a reliable native Rust command that performs Cloudflare-compatible latency and speed measurements from a terminal and returns both readable text and structured JSON.

The MVP prioritizes measurement correctness, transparent limitations, deterministic statistics, and low memory use. It does not prioritize customization, dashboards, packet-loss infrastructure, or graphical output.

## Included

### Measurement engine

- Requests to `https://speed.cloudflare.com/__down` and `https://speed.cloudflare.com/__up`.
- Cloudflare's published default measurement order for:
  - initial latency estimation;
  - initial 100 KB download estimation;
  - 20 unloaded-latency probes;
  - staged download measurements;
  - staged upload measurements.
- Median latency reduction (`0.5` percentile).
- 90th-percentile bandwidth reduction (`0.9` percentile).
- Jitter as the mean absolute difference between consecutive latency points.
- Loaded-latency probes during download and upload.
- A 400 ms loaded-latency probe interval.
- Maximum 20 retained loaded-latency points per direction.
- Minimum 10 ms request duration for inclusion in final bandwidth reduction.
- Minimum 250 ms duration for every transfer in a payload-size group before retaining that group's loaded-latency points.
- Strictly-greater-than-1000 ms minimum group duration threshold for skipping later groups in the same direction.
- 10 ms server-time fallback when no usable `Server-Timing` value exists.
- Request and stream timeouts.
- Ctrl+C cancellation.

### CLI

- Plain line-oriented terminal output.
- `--ipv4` and `--ipv6`.
- `--json`.
- `--no-download` and `--no-upload`.
- `--no-loaded-latency`.
- `--timeout`.
- `--quiet`.
- Standard help and version flags.

### Results

- Unloaded latency and jitter.
- Download and upload bandwidth.
- Download-loaded and upload-loaded latency and jitter when available.
- Individual raw points in JSON.
- Total test duration.
- Payload bytes uploaded and downloaded.
- Negotiated HTTP version when available.
- Packet loss represented as unavailable/null.

### Engineering

- Tokio asynchronous runtime.
- reqwest HTTP client with rustls TLS.
- Clap argument parsing.
- Serde JSON serialization.
- thiserror error types.
- Unit tests for statistics and scheduling.
- Local mock HTTP integration tests.
- CI checks for formatting, Clippy, tests, and release builds.

## Excluded from MVP

- TURN/WebRTC packet-loss measurement.
- Cloudflare AIM scores.
- Custom measurement-plan files.
- CSV output.
- Historical result storage.
- Result upload or sharing URLs.
- Full-screen TUI or interactive charts.
- Manual data-center selection.
- Multi-provider support.
- Proxy configuration beyond standard environment behavior.
- HTTP/3 as a release requirement; reqwest's HTTP/3 path is currently an unstable opt-in and must not block `0.1.0`.
- A background daemon or scheduled runner.

## Default measurement plan

Packet loss remains in the upstream sequence for traceability but is skipped by the MVP runner with an explicit unsupported result.

| Order | Type | Payload / packets | Count | MVP behavior |
|---:|---|---:|---:|---|
| 1 | Latency | 0 B | 1 | Run initial estimate |
| 2 | Download | 100,000 B | 1 | Run; bypass finish gate |
| 3 | Latency | 0 B | 20 | Run |
| 4 | Download | 100,000 B | 9 | Run unless direction finished |
| 5 | Download | 1,000,000 B | 8 | Run unless direction finished |
| 6 | Upload | 100,000 B | 8 | Run unless direction finished |
| 7 | Packet loss | 1,000 UDP packets | — | Skip; report unavailable |
| 8 | Upload | 1,000,000 B | 6 | Run unless direction finished |
| 9 | Download | 10,000,000 B | 6 | Run unless direction finished |
| 10 | Upload | 10,000,000 B | 4 | Run unless direction finished |
| 11 | Download | 25,000,000 B | 4 | Run unless direction finished |
| 12 | Upload | 25,000,000 B | 4 | Run unless direction finished |
| 13 | Download | 100,000,000 B | 3 | Run unless direction finished |
| 14 | Upload | 50,000,000 B | 3 | Run unless direction finished |
| 15 | Download | 250,000,000 B | 2 | Run unless direction finished |

## Acceptance criteria

### AC-1: default execution

Given working Internet access, `cfbench` runs without required arguments and prints a final summary.

### AC-2: schedule fidelity

A deterministic scheduler test confirms that the exact table above is used and that later same-direction groups are skipped after all requests in a group complete with a minimum adjusted duration strictly greater than 1000 ms.

### AC-3: streaming downloads

A 250 MB local mock response can be consumed without retaining the complete response body. Peak memory must remain bounded independently of payload size, excluding network-library buffers.

### AC-4: reusable uploads

Upload requests stream or reuse generated data and do not allocate a fresh payload-sized buffer for every measurement.

### AC-5: statistics

Known fixtures produce expected latency percentile, bandwidth percentile, and jitter values.

### AC-6: loaded latency

Loaded probes run concurrently with eligible transfers, use a 400 ms throttle, and retain at most the latest 20 points per direction.

### AC-7: JSON cleanliness

`cfbench --json` writes one parseable JSON document to stdout. No progress lines are mixed into stdout.

### AC-8: IP family

`--ipv4` prevents IPv6 connections and `--ipv6` prevents IPv4 connections. Passing both is rejected.

### AC-9: failure handling

A timeout or interrupted stream produces a stage-specific error and no panic. Completed points remain available in partial JSON or diagnostics.

### AC-10: disclosure

Help text and README identify `cfbench` as unofficial and describe native/browser timing differences.

## Suggested implementation milestones

1. Project skeleton, result types, CLI contract, and statistics.
2. HTTP client, latency probe, and server-timing parser.
3. Streaming download and upload measurements.
4. Measurement scheduler and adaptive early stopping.
5. Loaded-latency concurrency and cancellation.
6. Text and JSON output.
7. Local integration tests and cross-platform CI.
8. Live comparison runs and release documentation.

## Definition of done

The MVP is done when all acceptance criteria pass, release binaries can be built for the supported platforms, and the compatibility document accurately distinguishes exact upstream rules from native approximations.
