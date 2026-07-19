# cfbench Test Strategy

- Target: MVP `0.1.0`
- Date: 2026-07-19

## 1. Objectives

The test suite must prove that `cfbench`:

- applies the intended measurement schedule and reductions;
- streams large transfers without unbounded memory growth;
- handles timing and concurrency deterministically where possible;
- emits stable text and JSON contracts;
- reports request progress without blocking measurements or contaminating
  stdout;
- fails safely under realistic network errors;
- remains explicit about areas that cannot be exactly reproduced from a browser.

Live Internet speed values are inherently variable. CI must not assert a particular public bandwidth or latency value.

## 2. Test layers

### 2.1 Unit tests

Pure functions and isolated state machines:

- percentile reduction and edge cases;
- median behavior;
- 90th-percentile behavior;
- jitter calculation;
- duration adjustment and clamping;
- bps calculation;
- server-timing parsing;
- measurement-plan construction;
- same-direction finish/skip state;
- loaded-point retention at 20;
- JSON serialization;
- byte and unit formatting.

### 2.2 Component tests

Async units with controlled clocks or local channels:

- loaded-latency probe lifecycle;
- cancellation when a transfer finishes;
- no leaked Tokio tasks;
- 400 ms throttling using paused Tokio time;
- 250 ms loaded-transfer eligibility;
- request timeout mapping;
- partial-result accumulation.
- bounded progress backpressure and closed-receiver behavior;
- progress-renderer write failure cancellation and task joining.

### 2.3 Local HTTP integration tests

Use a local test server built with a minimal Hyper/Axum test fixture. The production client remains reqwest.

The server must support:

- exact-size streamed downloads;
- delayed headers;
- delayed body chunks;
- upload body counting;
- configurable `Server-Timing` headers;
- malformed headers;
- non-2xx status codes;
- early connection close;
- response-body size mismatch;
- hanging requests for timeout tests;
- HTTP/1.1 and, where practical, HTTP/2.

The release suite also includes a compact end-to-end plan that crosses the
actual `Runner` and `ReqwestTransport` boundary against this fixture. It must
produce reducible unloaded-latency, download, and upload summaries and record
zero unexpected request shapes. The compatible fixture URL remains test-only;
the public CLI does not expose an endpoint/provider flag.

### 2.4 CLI integration tests

Process-level CLI tests cover parser, help, version, invalid flags, and exit
code `2` for argument errors. Because the MVP intentionally has no endpoint
override, runtime network output is tested at the app boundary rather than by
pointing the compiled binary at a local fixture.

App-boundary tests assert:

- JSON-only stdout;
- exact line-oriented progress on stderr;
- `--quiet` and `--json` suppression of every progress line while diagnostics
  remain visible;
- exit codes;
- partial result behavior;
- no ANSI cursor-control sequences.

The compiled-binary tests use `assert_cmd`; app-boundary tests use in-memory
writers and scripted transports.

### 2.5 Live tests

Live tests target `speed.cloudflare.com` and are ignored by default.

They verify only broad invariants:

- endpoint reachability;
- requested download size is received;
- upload succeeds;
- non-negative timings;
- at least one valid latency point;
- summary values are finite and positive when the direction is enabled.
- the 100 MB download accepts production's normalized same-origin request
  context at the response-header boundary without reading the response body.

They must never be required for ordinary CI because they consume data and depend on external networking.

The MVP live set includes a zero-byte latency probe, an exact-size 65,536-byte
download, and a 65,536-byte upload. A separate regression guard sends the 100 MB
GET but drops the response immediately after headers, so it does not consume
the advertised body. Run the live set explicitly:

```text
cargo test --test live_cloudflare -- --ignored
```

No live test uses the default plan or requests the 250 MB download group.

Run only the request-context regression with:

```text
cargo test --lib transport::reqwest_transport::tests::live_large_download_accepts_browser_request_context -- --ignored --exact
```

The guard covers the HTTP 403 observed on 2026-07-19 when the same 100 MB GET
omitted both `Referer` and `Origin`. It reuses production request construction,
same-origin headers, absolute timeout, and explicit no-retry client policy while
remaining inside the transport's `#[cfg(test)]` module.

## 3. Critical test cases

### Statistics

1. Empty input returns unavailable, not zero.
2. One latency point produces latency but no jitter.
3. Consecutive identical points produce zero jitter.
4. Unsorted points are reduced correctly.
5. Non-finite values are rejected before reduction.
6. The percentile algorithm matches upstream fixtures exactly.

The Task 1 deterministic fixtures additionally cover invalid percentile
fractions, non-finite rejection, measurement-order-sensitive jitter, and zero
jitter for identical consecutive values. Finite extreme-value fixtures verify
that overflow cannot escape as a non-finite summary.

### Timing

1. Server time is subtracted from TTFB and transfer duration.
2. Missing server time uses 10 ms.
3. Server time greater than measured duration clamps safely.
4. Zero adjusted duration cannot produce infinity.
5. Monotonic timing is used regardless of wall-clock changes.

### Scheduling

1. The 100 KB initial download always runs because it bypasses the finish gate.
2. A qualifying download marks only download as finished.
3. Later upload groups still run after download finishes.
4. A group with any request at or below 1000 ms does not finish the direction.
5. Disabled download/upload groups are skipped without corrupting ordering.
6. The packet-loss slot produces an unavailable result without network activity.

The static plan compatibility fixture runs before scheduler implementation and
pins the upstream commit, all 15 ordered entries, configuration defaults, and
filtering of both transfer directions while retaining packet-loss metadata.

### Streaming and memory

1. Download chunks are consumed incrementally.
2. A 250 MB logical response does not require a 250 MB allocation.
3. Upload generation produces exactly N bytes.
4. Repeated upload measurements do not retain prior payload buffers.
5. Cancellation drops active streams promptly.

### Loaded latency

1. Probe task and transfer task run concurrently.
2. Probes stop when the transfer ends.
3. A payload-size group with any transfer below 250 ms does not contribute qualifying loaded context.
4. The latest 20 points are retained.
5. Download and upload loaded points remain separate.
6. Probe failures are recorded or discarded according to a defined rule without failing the bandwidth transfer automatically.

### Progress

1. Every accepted unloaded-latency and transfer point emits the documented
   phase/group counter and value.
2. Converted loaded probes use monotonic direction-local counters and may
   interleave with transfer progress.
3. Failures, adaptive stopping, and packet-loss unavailability use exact safe
   line forms without URLs, response bodies, ANSI escapes, or carriage returns.
4. A full or closed bounded channel never blocks measurement work or alters
   stored results.
5. Ordinary text writes progress only to stderr; `--quiet` and `--json` suppress
   it completely while final output and diagnostics retain their contracts.

### Error handling

1. DNS failure.
2. Connect timeout.
3. Header timeout.
4. Body-stall timeout.
5. TLS failure.
6. HTTP 429, 500, and 503.
7. Truncated download.
8. Upload connection reset.
9. Ctrl+C during latency, download, and upload.
10. Malformed `Server-Timing`.
11. HTTP 403 large-download request-context regression, ignored and live only.

## 4. Property tests

Property-based tests are recommended for the statistics and scheduling modules.

Potential invariants:

- a percentile result lies between the minimum and maximum finite inputs;
- jitter is never negative;
- increasing every point by a constant increases the percentile by that constant;
- total scheduled payload never increases after a direction is marked finished;
- retention never stores more than 20 loaded-latency points;
- serialized and deserialized results preserve all numeric fields within floating-point representation.

Suggested crate: `proptest`, added only if it provides clear value after deterministic fixtures are complete.

## 5. Golden files

Maintain reviewed golden fixtures for:

- text summary output;
- complete JSON output;
- partial JSON output;
- packet-loss unavailable state;
- IPv4/IPv6 metadata;
- representative upstream measurement plan.

Golden tests must normalize version, timestamps, and durations that vary per run.

## 6. Cross-platform matrix

Minimum CI matrix:

| OS | Architecture | Checks |
|---|---|---|
| Ubuntu | x86_64 | fmt, Clippy, unit, integration, release build |
| macOS | arm64 runner when available | unit, integration, release build |
| Windows | x86_64 | unit, integration, release build |

Additional release builds:

- Linux x86_64 GNU
- Linux aarch64 GNU
- macOS x86_64
- macOS aarch64
- Windows x86_64 MSVC

A musl build should be evaluated after TLS and DNS behavior are validated; it is not an MVP acceptance requirement.

GitHub Actions runs tests and a release build on Ubuntu, macOS, and Windows.
The Ubuntu job additionally enforces `cargo fmt --check` and Clippy with all
warnings denied. Ignored live tests are compiled but are not executed in CI.

## 7. Static quality gates

Required before merge:

```text
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
cargo build --release
```

Recommended:

- `cargo deny check` for licenses, bans, advisories, and sources;
- `cargo audit` where advisory availability permits;
- minimum supported Rust version build after an MSRV is declared.

## 8. Performance checks

Use local fixtures rather than public endpoints.

Track:

- peak resident memory during a streamed 250 MB download;
- CPU cost while consuming high-throughput local data;
- allocation count for repeated upload payloads where tooling permits;
- overhead of progress reporting;
- cancellation latency.

The tool is a network benchmark, so client-side CPU or output overhead must not become the limiting factor on common gigabit hardware.

An ignored local test streams the full 250,000,000-byte response using 64 KiB
fixture chunks without allocating a payload-sized body. Run it explicitly under
a platform memory tool, for example:

```text
# Linux
/usr/bin/time -v cargo test --release --test transport local_250_mb_download_streams_in_bounded_chunks -- --ignored --exact

# macOS
/usr/bin/time -l cargo test --release --test transport local_250_mb_download_streams_in_bounded_chunks -- --ignored --exact
```

Passing the test proves exact streaming byte count, not a peak-memory bound;
record the tool's maximum resident-set output separately. That measurement is
outstanding for `0.1.0` release validation.

## 9. Upstream parity workflow

At each Cloudflare Speedtest release:

1. Diff public configuration defaults and measurement order.
2. Diff percentile and result-reduction logic.
3. Diff server-timing parsing.
4. Diff loaded-latency scheduling.
5. Update compatibility fixtures and the baseline version.
6. Document changes in `CHANGELOG.md` when they affect results.

## 10. Release evidence

A release candidate should include:

- CI links for all target systems;
- the upstream baseline commit or release tag;
- deterministic parity fixture results;
- memory measurement for the largest download group;
- a small set of paired browser/native observations;
- known differences and unsupported features.

Current evidence status:

- upstream baseline and deterministic compatibility fixtures: available;
- local unit, integration, and release builds on the development host: available;
- executed Ubuntu/macOS/Windows CI matrix: outstanding until the workflow runs;
- peak-memory measurement for the ignored 250 MB fixture: outstanding;
- paired browser/native observations: outstanding;
- live default-plan and loaded-latency evidence: outstanding.
- ignored 100 MB header-only request-context coverage: available, with manual
  live execution required because external service behavior is not a local
  quality gate.

The repository must not be described as `0.1.0` release-ready until the
outstanding release evidence is collected and reviewed.
