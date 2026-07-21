# cfbench MVP Scope

- Target version: `0.1.0`
- Date: 2026-07-19

## MVP objective

Deliver a reliable native Rust command that performs Cloudflare-compatible latency and speed measurements from a terminal and returns both readable text and structured JSON.

The MVP prioritizes measurement correctness, transparent limitations, deterministic statistics, and low memory use. It does not prioritize customization, dashboards, packet-loss infrastructure, or graphical output.

## Included

### Measurement engine

- Requests to `https://speed.cloudflare.com/__down` and `https://speed.cloudflare.com/__up`.
- Normalized same-origin `Referer` on latency/download GETs and normalized
  `Referer` plus scheme-and-authority-only `Origin` on upload POSTs.
- At most one transport attempt per scheduled measurement; HTTP failures are
  not retried.
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
- One bounded `/meta` request after the complete timed plan, using the same
  client and strict address-family policy without retries or pre-test warming.

### CLI

- Plain line-oriented terminal output.
- Individual request progress on stderr with phase/group counters, safe failure
  categories, one adaptive-stop notice per direction, and an explicit
  packet-loss-unavailable notice.
- A bounded nonblocking progress channel that drops progress rather than
  perturbing measurement timing when stderr is slow.
- `--ipv4` and `--ipv6`.
- `--json`.
- `--no-download` and `--no-upload`.
- `--no-loaded-latency`.
- `--no-metadata`.
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
- Default Cloudflare-reported public IP, ASN, network organization, approximate
  client location, and edge colo/location, with an explicit privacy opt-out.
- One RFC 3339 UTC run-start timestamp and a Unix-millisecond completion
  timestamp on every accepted raw point.
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

The optional `/meta` enrichment request is not a measurement-plan entry. When
enabled (the default), it runs exactly once only after the 15 ordered plan
entries have finished and all loaded probes have stopped. Its response bytes
and elapsed time are excluded from download/upload usage and
`usage.duration_ms`. `--no-metadata` omits it entirely.

The default schedule is exposed as a versioned `MeasurementPlan` derived from
a compile-time fixture. `MeasurementPlan::for_config` creates the run-specific
view without mutating the baseline: disabled download or upload steps are
removed, while latency and packet-loss metadata retain their source order. The
default `RunConfig` uses automatic IP-family selection, enables both transfer
directions, loaded latency, and post-plan metadata, and applies a 30-second
per-request timeout.

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

The static compatibility test also pins the upstream commit, all 15 plan
entries, direction filtering, and the initial bypass flag independently of the
runner state-machine tests.

### AC-3: streaming downloads

A 250 MB local mock response can be consumed without retaining the complete response body. Peak memory must remain bounded independently of payload size, excluding network-library buffers.

### AC-4: reusable uploads

Upload requests stream or reuse generated data and do not allocate a fresh payload-sized buffer for every measurement.

### AC-5: statistics

Known fixtures produce expected latency percentile, bandwidth percentile, and jitter values.

### AC-6: loaded latency

Loaded probes run concurrently with eligible transfers, use a 400 ms throttle, and retain at most the latest 20 points per direction.

### AC-7: JSON cleanliness

`cfbench --json` writes one parseable JSON document to stdout and suppresses all
progress. Ordinary text mode uses the documented exact stderr lines;
`--quiet` suppresses all progress but preserves final results and diagnostics.
Schema version 1 remains one document with additive `started_at`,
`metadata_status`, nullable `metadata`, and per-point
`measured_at_unix_ms` fields.

### AC-8: IP family

`--ipv4` prevents IPv6 connections and `--ipv6` prevents IPv4 connections. Passing both is rejected.

### AC-9: failure handling

A timeout or interrupted stream produces a stage-specific error and no panic. Completed points remain available in partial JSON or diagnostics.

### AC-10: disclosure

Help text and README identify `cfbench` as unofficial and describe native/browser timing differences.

### AC-11: large-request context

The 100 MB download uses the same normalized request context as ordinary
production GETs, receives a successful header status from Cloudflare when the
endpoint is available, and can drop the response immediately without consuming
the body. The ignored live guard protects the HTTP 403 regression observed on
2026-07-19 and is not an ordinary CI gate.

### AC-12: metadata privacy, ordering, and failure isolation

A deterministic local test proves metadata collection defaults on, occurs once
after the last plan operation, does not change plan usage or duration, and
retains nullable leaves from a bounded valid object. `--no-metadata` performs
zero metadata requests and serializes `metadata_status: "disabled"` with
`metadata: null`. Enabled retrieval failure serializes `unavailable`, emits one
redacted diagnostic, preserves completed points, and does not change a
measurement-derived success or failure status.

Text tests cover complete, partial, unavailable, and disabled metadata without
dangling punctuation. JSON raw points remain in latency/direction/loaded
direction arrays; `requested_bytes` is the bandwidth group key rather than a
duplicated grouped representation. Every accepted point has a nondecreasing
completion timestamp that does not participate in reductions.

An ignored live `/meta` guard checks only a nonempty public-IP field, a positive
32-bit ASN, and a three-letter uppercase colo. It must not print or preserve the
returned personal/network values in assertions or fixtures.

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
