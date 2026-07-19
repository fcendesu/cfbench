# cfbench Architecture and Design Specification

- Status: Proposed and ready for implementation planning
- Date: 2026-07-19
- Product: `cfbench`
- Language: Rust

## 1. Design summary

`cfbench` is a small command-line application with a transport-independent measurement core. The CLI parses configuration, constructs a Cloudflare measurement plan, executes it through a reqwest-backed client, reduces raw points into summaries, and renders text or JSON.

The design separates network I/O from statistics so that deterministic logic can be tested without live Internet access and the HTTP implementation can be replaced if more precise timing is required later.

## 2. Architecture

```text
CLI arguments
    |
    v
RunConfig ---------> OutputMode
    |
    v
MeasurementPlan
    |
    v
Runner --------------------------+
    |                             |
    v                             v
CloudflareTransport        LoadedLatencyController
    |                             |
    +-------------+---------------+
                  |
                  v
             RawResults
                  |
                  v
             ResultReducer
                  |
                  v
              Summary
             /       \
        TextOutput  JsonOutput
```

## 3. Module boundaries

Suggested source layout:

```text
src/
├── main.rs
├── app.rs
├── cli.rs
├── config.rs
├── error.rs
├── plan.rs
├── runner.rs
├── cancellation.rs
├── transport/
│   ├── mod.rs
│   ├── reqwest_transport.rs
│   ├── server_timing.rs
│   └── resolver.rs
├── measurement/
│   ├── mod.rs
│   ├── latency.rs
│   ├── download.rs
│   ├── upload.rs
│   └── loaded_latency.rs
├── results/
│   ├── mod.rs
│   ├── point.rs
│   ├── summary.rs
│   └── reduce.rs
├── statistics/
│   ├── mod.rs
│   ├── percentile.rs
│   └── jitter.rs
└── output/
    ├── mod.rs
    ├── text.rs
    └── json.rs
```

### `cli`

Owns Clap definitions and conversion into validated `RunConfig`. It contains no network logic.

### `app`

Owns the testable command boundary: signal installation and cooperative runner cancellation, progress routing, one-result stdout rendering, terminal diagnostics, and outcome-to-exit-status mapping. `main.rs` only wires production streams, signal handling, transport, and plan construction into this boundary.

### `plan`

Defines `MeasurementStep` and the immutable default plan. It also applies user disables such as `--no-upload` without changing the source baseline.

### `transport`

Provides a narrow async interface for zero-byte latency requests, downloads, and uploads. Production uses reqwest; tests use a local server rather than a large fake transport wherever streaming behavior matters.

### `measurement`

Converts transport observations into raw points. It owns monotonic timestamps, byte counts, server-time adjustment, and loaded probe coordination.

### `runner`

Executes the ordered plan, tracks whether download or upload has met the finish threshold, accumulates partial results, and propagates cancellation.

### `statistics` and `results`

Pure, synchronous logic. These modules reduce finite validated points and must not depend on reqwest or terminal output.

### `output`

Renders completed or partial results. JSON structures are versioned and serialized from owned result models rather than ad-hoc maps.

## 4. Core types

Illustrative interfaces:

```rust
pub enum IpMode {
    Auto,
    V4Only,
    V6Only,
}

pub struct RunConfig {
    pub ip_mode: IpMode,
    pub request_timeout: Duration,
    pub no_download: bool,
    pub no_upload: bool,
    pub no_loaded_latency: bool,
}

pub enum MeasurementStep {
    Latency { packets: u32 },
    Download { bytes: u64, count: u32, bypass_finish: bool },
    Upload { bytes: u64, count: u32, bypass_finish: bool },
    PacketLossUnsupported { packets: u32, responses_wait_ms: u32 },
}

pub struct MeasurementPlan {
    pub upstream_version: &'static str,
    pub upstream_commit: &'static str,
    pub steps: Vec<MeasurementStep>,
}

pub struct TimingObservation {
    pub ttfb: Duration,
    pub total: Duration,
    pub server_time: Duration,
    pub payload_bytes: u64,
    pub http_version: Option<String>,
}

pub struct BandwidthPoint {
    pub direction: Direction,
    pub requested_bytes: u64,
    pub payload_bytes: u64,
    pub duration_ms: f64,
    pub adjusted_duration_ms: f64,
    pub ping_ms: f64,
    pub server_time_ms: f64,
    pub bps: f64,
}
```

Concrete names may change during implementation, but boundaries and responsibilities should remain.

The Task 1 implementation uses these concrete names. The default plan copies a
compile-time 15-step fixture into an owned plan so configuration filtering can
produce a separate ordered plan without changing the source baseline. The
default request timeout is 30 seconds. Transfer steps expose their `Direction`;
latency and unsupported packet-loss steps have no transfer direction.

## 5. Data flow

1. Clap parses arguments.
2. `RunConfig::try_from(cli)` rejects conflicting or invalid options.
3. `default_cloudflare_plan()` returns the versioned upstream baseline.
4. The plan is filtered for disabled directions while preserving metadata.
5. `ReqwestTransport` is created with TLS, compression disabled, timeout policy, and IP-family behavior.
6. `Runner` executes steps sequentially.
7. Each bandwidth request may run with a loaded-latency controller.
8. Valid and failed observations are appended to raw results.
9. Finish-state is updated independently for download and upload.
10. The reducer computes summary values from eligible points.
11. The chosen output renderer writes stdout; progress and errors use stderr.

## 6. HTTP client design

Build one `reqwest::Client` per run.

Recommended behavior:

- rustls TLS;
- response decompression disabled;
- redirects disabled or tightly limited because redirects invalidate endpoint timing assumptions;
- default request headers include a project user agent and `Accept-Encoding: identity`;
- no retries;
- no Tower middleware;
- timeout enforced at the operation and stream level;
- connection pool reused through the run.

The dependency version should be selected and locked when implementation begins rather than hard-coded in this design document.

## 7. Download streaming

The download function:

1. Creates the URL with the requested `bytes` query.
2. Starts `Instant`.
3. Sends the request and records response-header arrival.
4. Validates the status.
5. Parses server timing.
6. Iterates body chunks, incrementing a `u64` counter.
7. Records body completion.
8. Validates actual payload size against requested size.
9. Returns a `TimingObservation`.

It never concatenates body chunks.

## 8. Upload generation

The upload source must emit exactly N bytes without requiring one fresh N-byte allocation per request.

Preferred MVP approach:

- keep one moderate immutable chunk, for example 64 KiB;
- create an async stream that yields the chunk repeatedly and a final slice-sized chunk logically equivalent in length;
- wrap it as a reqwest body;
- ensure the stream can be recreated for each request;
- avoid random generation in the timing path.

An alternative is a reusable `Bytes` buffer for small groups and a stream for larger groups. One implementation should be chosen based on benchmark simplicity, not premature optimization.

## 9. Loaded-latency coordination

A payload-size group and its probes share a cancellation token scoped to that group.

Conceptual flow:

```text
create group token
spawn loaded probe loop
run every sequential transfer in the group
cancel group token
await probe loop shutdown
if every transfer duration in the payload-size group >= 250 ms:
    retain qualifying probe points
else:
    discard or mark non-qualifying probe points
```

Use `tokio_util::sync::CancellationToken` if accepted as a dependency; otherwise use a watch channel with explicit ownership.

Do not hold locks across network awaits. Points can be returned from the probe task or sent through an MPSC channel.

## 10. Scheduler and finish state

Runner state:

```text
download_finished: bool
upload_finished: bool
```

For each bandwidth group:

- skip when its direction is disabled;
- skip when its direction is finished;
- execute up to `count` requests;
- keep one loaded-latency probe loop active across the group's sequential requests;
- append successful points and record failures;
- after completing every request in a group, mark its direction finished when the group's minimum adjusted duration is strictly greater than 1000 ms and the group does not bypass the finish gate;
- continue with the next plan step, including the opposite direction.

Every request in the current group completes before the runner evaluates the strict minimum-duration finish condition, matching the pinned upstream source.

## 11. Statistics

Statistics accept only finite validated `f64` values.

### Percentile

Port the exact upstream percentile/reduction algorithm after source inspection. Do not assume a library percentile function is equivalent because interpolation and index selection vary.

The pinned implementation sorts finite inputs with `f64::total_cmp` and uses
linear interpolation at `(len - 1) * fraction`. Empty input, non-finite values,
fractions outside the inclusive range `0..=1`, and non-finite computed results
return `None`.

### Jitter

For points in measurement order:

```text
sum(abs(points[i] - points[i-1])) / (len(points) - 1)
```

Fewer than two points, any non-finite point, or a non-finite computed result
returns `None`.

### Bandwidth eligibility

Only points with adjusted transfer duration at or above 10 ms enter the final bandwidth percentile.

## 12. Cancellation and timeouts

A top-level cancellation token is cancelled by Ctrl+C.

Every request must respond to:

- top-level cancellation;
- per-request timeout;
- transfer-scoped cancellation for loaded probes.

A cancelled run returns partial results when available and a non-zero exit status. No detached task may continue after output is rendered.

## 13. Error model

Use a typed error enum with contextual variants such as:

- configuration;
- DNS/connect/TLS;
- request timeout;
- HTTP status;
- body stream;
- payload-size mismatch;
- malformed required response;
- cancellation;
- result unavailable;
- serialization/output.

Malformed optional `Server-Timing` is not fatal; it uses the fallback and records a diagnostic field where useful.

## 14. Output design

### Text

- Stable labels and units.
- Mbps uses decimal SI (`1 Mbps = 1,000,000 bps`).
- Payload MB should be clearly defined as decimal MB, or rendered with a byte-format utility that states its convention.
- Progress is line-oriented and written to stderr.
- No alternate screen or cursor movement.

### JSON

- One document on stdout.
- Integer bps and byte counts where possible.
- Millisecond measurements may be floating-point.
- Missing values are `null`, not magic values.
- Raw points include enough data to reproduce reductions.
- Schema version begins at `1`.

## 15. Security and privacy

- No project telemetry.
- No result collection backend.
- Never log TURN credentials if added later.
- Do not accept arbitrary endpoint overrides in MVP unless the security and semantics are clearly defined.
- Limit maximum timeout and future custom payload settings to prevent accidental resource abuse.
- Use a clear user agent identifying the project and version.

## 16. Testing design

- Pure unit tests for reductions and state transitions.
- Paused Tokio time for probe scheduling.
- Local HTTP fixture for delay, size, streaming, status, timeout, and truncation behavior.
- CLI process tests for output contracts.
- Ignored live tests for endpoint compatibility.
- Golden plan fixture pinned to the upstream baseline.

See `docs/TEST_STRATEGY.md` for the full matrix.

## 17. Dependency outline

Likely production dependencies:

```text
tokio
reqwest
clap
serde
serde_json
thiserror
futures-util or futures-core
tokio-util (optional, for CancellationToken)
bytes
```

Likely development dependencies:

```text
assert_cmd (optional)
predicates (optional)
axum or hyper for the local fixture
proptest (optional)
```

Keep dependencies minimal, but do not implement fragile replacements for well-tested cancellation, parsing, or CLI behavior solely to reduce crate count.

## 18. Upstream validation resolved before implementation

Source inspection of Cloudflare Speedtest `v1.11.0` at commit `cfc99a74fd8d5c2121d319aeb7894c6246202c65` established:

1. Percentiles use sorted linear interpolation at index `(len - 1) * percentile`.
2. `Server-Timing` parsing takes the first finite decimal `dur` token matched at the start of the header or after a semicolon; missing or unusable values use the configured 10 ms estimate.
3. Upstream adds only the stable `bytes` query, plus `during` for loaded probes; it does not add a random cache-buster.
4. Every request in the current group completes, and finish state uses the strict `minimum_group_duration > 1000 ms` comparison.
5. Loaded probing starts after 20 ms, then throttles successive requests by 400 ms.
6. The later 20-packet latency phase replaces the initial one-packet estimate in public results.
7. Browser uploads use a generated string body, which implies the Fetch string-body content type. The native client sends a streaming body with `Content-Type: text/plain;charset=UTF-8` and an exact `Content-Length` to preserve the public request semantics without payload-sized allocation.

The implementation plan should schedule these checks before the corresponding code is finalized.

## 19. Rejected alternatives

### Headless browser wrapper

Rejected because it makes the CLI large, slow to install, and dependent on Chromium. It would be closer to browser timing but conflicts with the native single-binary goal.

### Tower-first HTTP stack

Rejected because Tower is middleware infrastructure rather than the required HTTP client. Generic retry, buffering, and rate-limit layers also risk altering measurements.

### Hyper-first implementation

Deferred because reqwest provides sufficient MVP capabilities with much less plumbing. The design keeps the transport isolated so Hyper can replace it if precise timing hooks become mandatory.

### ICMP packet loss

Rejected as a replacement for Cloudflare's TURN/WebRTC packet-loss semantics.

## 20. References

- Cloudflare Speedtest: <https://github.com/cloudflare/speedtest>
- Cloudflare defaults: <https://github.com/cloudflare/speedtest/blob/main/README.md>
- PerformanceResourceTiming: <https://developer.mozilla.org/en-US/docs/Web/API/PerformanceResourceTiming>
- reqwest: <https://docs.rs/reqwest/latest/reqwest/>
