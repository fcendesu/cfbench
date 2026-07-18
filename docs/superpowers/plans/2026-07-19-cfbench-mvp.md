# cfbench MVP Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `cfbench 0.1.0`, a native Rust CLI that executes Cloudflare Speedtest's `v1.11.0` latency, bandwidth, and loaded-latency methodology with text and versioned JSON output.

**Architecture:** Keep CLI parsing, immutable planning, reqwest I/O, measurement conversion, orchestration, pure reductions, result models, and rendering in separate modules. One reqwest client is reused for a run; bodies are streamed; a cancellation token scopes the run and every loaded-probe loop.

**Tech Stack:** Stable Rust 1.95, Tokio 1.53, reqwest 0.13 with rustls and streaming, Clap 4.6, Serde 1.0, serde_json 1.0, thiserror 2.0, futures-util 0.3, tokio-util 0.7, bytes 1.12; Axum 0.8 and assert_cmd 2.2 for tests.

## Global Constraints

- Upstream compatibility is pinned to Cloudflare Speedtest `v1.11.0`, commit `cfc99a74fd8d5c2121d319aeb7894c6246202c65`.
- Preserve the exact published measurement order; packet loss remains a plan step but serializes as unavailable with reason `turn_not_implemented`.
- Finish a direction only after every request in a payload-size group completes and the minimum adjusted group duration is strictly greater than `1000 ms`; the initial 100 KB download bypasses this gate.
- Use sorted linear interpolation at index `(len - 1) * percentile`; latency uses `0.5`, bandwidth uses `0.9`, and bandwidth eligibility requires adjusted duration at least `10 ms`.
- Loaded probing begins after `20 ms`, successive probe starts are at least `400 ms` apart, a group is eligible only when every transfer lasts at least `250 ms`, and only the latest `20` points per direction are retained.
- Use `std::time::Instant`, one reqwest client and pool per run, `Accept-Encoding: identity`, no transparent decompression, no redirects, no retries, and explicit request/header/body timeouts.
- Stream and count download bodies; stream uploads from one reusable moderate chunk without allocating a payload-sized body per request.
- Bandwidth uses the upstream 1.005 estimated-transfer-byte multiplier while results retain actual payload byte counts.
- IPv4-only and IPv6-only modes are strict. `--json` emits exactly one JSON document to stdout; progress and diagnostics use stderr; missing values serialize as `null`.
- Do not use `unwrap`, `expect`, or `panic!` in runtime paths. Preserve partial results on failure or cancellation and await all spawned tasks.
- The MVP has no TUI, TURN packet loss, AIM score, CSV, custom provider, arbitrary endpoint flag, history, daemon, or HTTP/3 requirement.

---

### Task 1: Crate skeleton, configuration, immutable plan, and pure statistics

**Files:**
- Create: `Cargo.toml`, `.gitignore`, `src/lib.rs`, `src/config.rs`, `src/plan.rs`, `src/statistics/mod.rs`, `src/statistics/percentile.rs`, `src/statistics/jitter.rs`
- Create: `tests/plan_compatibility.rs`, `tests/statistics.rs`
- Modify: `docs/MEASUREMENT_COMPATIBILITY.md`, `docs/MVP.md`, `docs/superpowers/specs/2026-07-19-cfbench-design.md`, `docs/TEST_STRATEGY.md`

**Interfaces:**
- Produces `IpMode`, `RunConfig`, `Direction`, `MeasurementStep`, `MeasurementPlan`, `default_cloudflare_plan()`, `percentile()`, and `jitter()`.
- `MeasurementPlan::for_config(&RunConfig)` preserves source order and removes disabled direction steps without removing the unsupported packet-loss metadata step.

- [ ] **Step 1: Add failing plan and statistics tests**

```rust
#[test]
fn upstream_plan_matches_v1_11_0() {
    let plan = cfbench::plan::default_cloudflare_plan();
    assert_eq!(plan.upstream_commit, "cfc99a74fd8d5c2121d319aeb7894c6246202c65");
    assert_eq!(plan.steps.len(), 15);
    assert_eq!(plan.steps[1], MeasurementStep::Download { bytes: 100_000, count: 1, bypass_finish: true });
    assert_eq!(plan.steps[14], MeasurementStep::Download { bytes: 250_000_000, count: 2, bypass_finish: false });
}

#[test]
fn percentile_uses_upstream_linear_interpolation() {
    assert_eq!(percentile(&[10.0, 0.0, 30.0, 20.0], 0.5), Some(15.0));
    assert_eq!(percentile(&[0.0, 10.0, 20.0, 30.0], 0.9), Some(27.0));
    assert_eq!(percentile(&[], 0.5), None);
}

#[test]
fn jitter_requires_two_finite_points() {
    assert_eq!(jitter(&[10.0]), None);
    assert_eq!(jitter(&[10.0, 14.0, 12.0]), Some(3.0));
    assert_eq!(jitter(&[10.0, f64::NAN]), None);
}
```

- [ ] **Step 2: Run the focused tests and verify RED**

Run: `cargo test --test plan_compatibility --test statistics`

Expected: compilation fails because the crate modules and public functions do not exist.

- [ ] **Step 3: Implement the minimal domain types and pure functions**

```rust
pub fn percentile(values: &[f64], fraction: f64) -> Option<f64> {
    if values.is_empty() || !(0.0..=1.0).contains(&fraction) || values.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let index = (sorted.len() - 1) as f64 * fraction;
    let lower = index.floor() as usize;
    let upper = index.ceil() as usize;
    Some(sorted[lower] + (sorted[upper] - sorted[lower]) * index.fract())
}

pub fn jitter(values: &[f64]) -> Option<f64> {
    if values.len() < 2 || values.iter().any(|v| !v.is_finite()) { return None; }
    Some(values.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f64>() / (values.len() - 1) as f64)
}
```

Define the exact 15-step `const`-backed plan, configuration defaults (`30` second request timeout), and `no_download`/`no_upload` filtering. Add `[package]` metadata and narrow dependency features; do not add a binary until Task 5.

- [ ] **Step 4: Verify GREEN and formatting**

Run: `cargo fmt --check && cargo test --test plan_compatibility --test statistics`

Expected: both integration test targets pass with no warnings.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock .gitignore src tests docs README.md AGENTS.md
git commit -m "feat(core): add measurement plan and statistics"
```

### Task 2: Result models, timing conversion, and deterministic reductions

**Files:**
- Create: `src/results/mod.rs`, `src/results/point.rs`, `src/results/summary.rs`, `src/results/reduce.rs`, `src/measurement/mod.rs`, `src/measurement/timing.rs`
- Create: `tests/reductions.rs`, `tests/result_schema.rs`

**Interfaces:**
- Consumes `Direction`, `percentile`, and `jitter` from Task 1.
- Produces serializable `LatencyPoint`, `BandwidthPoint`, `RawResults`, `Summary`, `RunResult`, `PacketLossResult`, `TimingObservation`, `latency_point()`, `bandwidth_point()`, and `reduce()`.

- [ ] **Step 1: Add failing conversion and reduction tests**

```rust
#[test]
fn bandwidth_applies_server_adjustment_and_header_estimate() {
    let observation = TimingObservation::from_millis(30.0, 210.0, 10.0, 1_000_000, "HTTP/2");
    let point = bandwidth_point(Direction::Download, 1_000_000, observation).unwrap();
    assert_eq!(point.adjusted_duration_ms, 200.0);
    assert_eq!(point.bps, 40_200_000);
    assert_eq!(point.payload_bytes, 1_000_000);
}

#[test]
fn reducer_filters_short_bandwidth_points() {
    let raw = fixture_with_download_points([(9.99, 900_000_000), (10.0, 100_000_000), (20.0, 200_000_000)]);
    assert_eq!(reduce(&raw).download_bps, Some(190_000_000));
}

#[test]
fn unavailable_packet_loss_is_explicit() {
    let value = serde_json::to_value(RunResult::empty()).unwrap();
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["summary"]["packet_loss_ratio"], serde_json::Value::Null);
    assert_eq!(value["packet_loss"]["reason"], "turn_not_implemented");
}
```

- [ ] **Step 2: Run focused tests and verify RED**

Run: `cargo test --test reductions --test result_schema`

Expected: compilation fails because result and measurement types are missing.

- [ ] **Step 3: Implement models and reductions**

Store duration fields as finite `f64` milliseconds, payload fields as `u64`, final bps as rounded `u64`, and failures/diagnostics in stable arrays. Use:

```rust
const TRANSFER_OVERHEAD_FACTOR: f64 = 1.005;
const MIN_ADJUSTED_DURATION_MS: f64 = 0.01;

let adjusted = (observation.total.as_secs_f64() - observation.server_time.as_secs_f64())
    .max(MIN_ADJUSTED_DURATION_MS / 1000.0);
let bps = ((observation.payload_bytes as f64 * TRANSFER_OVERHEAD_FACTOR * 8.0) / adjusted).round();
```

The reducer uses only the later unloaded phase, filters bandwidth points below `10 ms`, keeps download/upload loaded sets separate, and trims eligible loaded points to the latest `20` before percentile and jitter calculation.

- [ ] **Step 4: Verify GREEN**

Run: `cargo fmt --check && cargo test --test reductions --test result_schema`

Expected: all conversion, reduction, serialization, finite-value, and empty-result tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/measurement src/results tests/reductions.rs tests/result_schema.rs
git commit -m "feat(results): add raw points and summary reductions"
```

### Task 3: Reqwest transport with strict family selection and streaming bodies

**Files:**
- Create: `src/error.rs`, `src/transport/mod.rs`, `src/transport/reqwest_transport.rs`, `src/transport/server_timing.rs`, `src/transport/upload_body.rs`
- Create: `tests/support/mod.rs`, `tests/transport.rs`, `tests/server_timing.rs`, `tests/upload_body.rs`

**Interfaces:**
- Produces `TransportError`, `ReqwestTransport::new(RunConfig)`, `latency(&CancellationToken)`, `download(bytes, during, &CancellationToken)`, and `upload(bytes, &CancellationToken)` returning transport-owned `TimingObservation` values.
- `stream_upload(bytes)` yields exactly `bytes` from reusable 64 KiB zero chunks and provides an exact content length.

- [ ] **Step 1: Add failing parser, streaming, and local HTTP tests**

```rust
#[test]
fn parses_cloudflare_server_duration() {
    assert_eq!(server_duration(Some("cfRequestDuration;dur=15.999794")), Duration::from_secs_f64(0.015999794));
    assert_eq!(server_duration(Some("edge;desc=x, cfRequestDuration;dur=8.5")), Duration::from_secs_f64(0.0085));
    assert_eq!(server_duration(Some("cfRequestDuration;dur=NaN")), Duration::from_millis(10));
}

#[tokio::test]
async fn upload_stream_emits_exact_length_without_payload_sized_buffer() {
    let (body, content_length) = stream_upload(150_000);
    assert_eq!(content_length, 150_000);
    assert_eq!(collect_chunk_lengths(body).await, vec![65_536, 65_536, 18_928]);
}

#[tokio::test]
async fn download_counts_streamed_bytes_and_rejects_truncation() {
    let server = FixtureServer::streaming_download(100_000, 8_192).await;
    let point = transport_for(server.url()).download(100_000, None, &CancellationToken::new()).await.unwrap();
    assert_eq!(point.payload_bytes, 100_000);
    assert!(transport_for(server.url()).download(100_001, None, &CancellationToken::new()).await.is_err());
}
```

- [ ] **Step 2: Run focused tests and verify RED**

Run: `cargo test --test server_timing --test upload_body --test transport`

Expected: compilation fails because the transport modules do not exist.

- [ ] **Step 3: Implement the client and streaming operations**

Build one client with redirects disabled, no gzip/brotli/deflate/zstd features, user agent `cfbench/<version>`, and `Accept-Encoding: identity`. In family-only modes bind the unspecified address of that family. Wrap header arrival and each body-stream poll in `tokio::select!` against cancellation and timeout. Reject non-success status and download length mismatch. For upload, set `Content-Type: text/plain;charset=UTF-8`, `Content-Length`, and consume the response body to EOF.

The parser follows upstream's decimal `dur` behavior but rejects non-finite values and returns the `10 ms` fallback. Error variants distinguish cancelled, header timeout, body timeout, HTTP status, body stream, and payload mismatch. No operation retries.

- [ ] **Step 4: Verify GREEN including family behavior**

Run: `cargo fmt --check && cargo test --test server_timing --test upload_body --test transport`

Expected: exact-size, truncation, malformed header, status, timeout, cancellation, upload counting, IPv4-only success, and IPv6-only-to-IPv4-fixture failure tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/error.rs src/transport tests/support tests/transport.rs tests/server_timing.rs tests/upload_body.rs Cargo.toml Cargo.lock
git commit -m "feat(transport): stream Cloudflare measurements"
```

### Task 4: Ordered runner, finish state, loaded probing, cancellation, and partial failures

**Files:**
- Create: `src/cancellation.rs`, `src/runner.rs`, `src/measurement/loaded_latency.rs`
- Create: `tests/runner.rs`, `tests/loaded_latency.rs`

**Interfaces:**
- Consumes `MeasurementPlan`, transport operations, conversion functions, and `RawResults`.
- Produces `Runner<T>::run(&CancellationToken) -> RunOutcome`, where `RunOutcome` always contains a `RunResult` and optionally a typed terminal error.
- A small internal `MeasurementTransport` trait permits deterministic runner tests without reqwest types escaping the transport boundary.

- [ ] **Step 1: Add failing state-machine and paused-time tests**

```rust
#[tokio::test]
async fn finish_uses_strict_minimum_after_whole_group() {
    let transport = ScriptedTransport::durations([1001.0, 1500.0, 1000.0]);
    let outcome = run_download_groups(transport, [(100_000, 3, false), (1_000_000, 1, false)]).await;
    assert_eq!(outcome.raw.download.len(), 4);
    let transport = ScriptedTransport::durations([1001.0, 1500.0, 1000.01]);
    let outcome = run_download_groups(transport, [(100_000, 3, false), (1_000_000, 1, false)]).await;
    assert_eq!(outcome.raw.download.len(), 3);
}

#[tokio::test(start_paused = true)]
async fn loaded_probe_starts_at_20ms_then_throttles_400ms() {
    let harness = LoadedHarness::new();
    let task = harness.spawn_probe_loop();
    tokio::time::advance(Duration::from_millis(19)).await;
    assert_eq!(harness.starts(), 0);
    tokio::time::advance(Duration::from_millis(1)).await;
    assert_eq!(harness.starts(), 1);
    tokio::time::advance(Duration::from_millis(399)).await;
    assert_eq!(harness.starts(), 1);
    tokio::time::advance(Duration::from_millis(1)).await;
    assert_eq!(harness.starts(), 2);
    harness.cancel_and_join(task).await;
}

#[tokio::test]
async fn failed_later_stage_preserves_completed_points() {
    let outcome = Runner::new(script_succeed_then_fail()).run(&CancellationToken::new()).await;
    assert!(outcome.error.is_some());
    assert!(!outcome.result.points.latency.is_empty());
}
```

- [ ] **Step 2: Run focused tests and verify RED**

Run: `cargo test --test runner --test loaded_latency`

Expected: compilation fails because the runner and probe coordinator do not exist.

- [ ] **Step 3: Implement sequential orchestration**

Execute all 15 plan entries in order; replace the initial unloaded latency vector when the 20-packet phase completes; skip packet loss without I/O; track download/upload finish booleans independently; complete all requests in a group before evaluating `min_duration > 1000.0`; and keep successful prior points when a later request fails.

For each bandwidth request, spawn one probe loop only when loaded latency is enabled. Delay first start by `20 ms`; after each probe completes wait until at least `400 ms` from its start; cancel and await the task when transfer completes or fails. Buffer its points locally, then associate them with the payload-size group. Retain the group's points only if all its successful transfers have adjusted durations `>= 250 ms`; trim direction results to the latest 20. Probe failures add diagnostics but do not fail an otherwise successful transfer.

- [ ] **Step 4: Verify GREEN and task shutdown**

Run: `cargo fmt --check && cargo test --test runner --test loaded_latency`

Expected: schedule, bypass, strict boundary, independent direction state, replacement latency phase, probe cadence, group eligibility, latest-20 retention, cancellation, and partial-result tests pass without leaked tasks.

- [ ] **Step 5: Commit**

```bash
git add src/cancellation.rs src/runner.rs src/measurement/loaded_latency.rs src/lib.rs tests/runner.rs tests/loaded_latency.rs
git commit -m "feat(runner): orchestrate adaptive measurements"
```

### Task 5: CLI, text output, JSON output, progress routing, and exit semantics

**Files:**
- Create: `src/cli.rs`, `src/output/mod.rs`, `src/output/text.rs`, `src/output/json.rs`, `src/main.rs`
- Create: `tests/cli.rs`, `tests/output.rs`
- Modify: `src/config.rs`, `src/error.rs`

**Interfaces:**
- `Cli` maps to validated `RunConfig`; Clap rejects `--ipv4 --ipv6` and timeout values outside `1..=300` seconds.
- `render_text(&RunResult) -> String` and `render_json(&RunResult) -> Result<String, OutputError>` are pure.
- Main writes progress only to stderr, exactly one final result to stdout, and exits `0` on success, `1` on failed/cancelled run, or Clap's `2` on invalid arguments.

- [ ] **Step 1: Add failing CLI and golden-contract tests**

```rust
#[test]
fn ip_family_flags_conflict() {
    Command::cargo_bin("cfbench").unwrap().args(["--ipv4", "--ipv6"]).assert().failure().code(2);
}

#[test]
fn help_discloses_unofficial_native_methodology() {
    Command::cargo_bin("cfbench").unwrap().arg("--help").assert().success()
        .stdout(predicate::str::contains("unofficial"))
        .stdout(predicate::str::contains("Cloudflare-compatible methodology"));
}

#[test]
fn render_json_is_one_document_with_nulls() {
    let rendered = render_json(&RunResult::empty()).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&rendered).unwrap();
    assert_eq!(parsed["schema_version"], 1);
    assert!(parsed["summary"]["download_bps"].is_null());
    assert_eq!(rendered.matches('{').count(), rendered.matches('}').count());
}
```

- [ ] **Step 2: Run focused tests and verify RED**

Run: `cargo test --test cli --test output`

Expected: the binary and renderers do not exist.

- [ ] **Step 3: Implement parsing, rendering, and main**

Use Clap derive with exactly the PRD flags. Render decimal Mbps and MB, negotiated family/version metadata, null/unavailable values, total payload counts, and duration. Install Ctrl+C handling before starting the runner. Always render partial results; after output, return the outcome's non-zero status. Never emit progress in JSON mode; `--quiet` suppresses progress only.

- [ ] **Step 4: Verify GREEN and stream separation**

Run: `cargo fmt --check && cargo test --test cli --test output`

Expected: help/version, conflict, timeout bounds, quiet behavior, no ANSI sequences, stable text labels, parseable JSON, and stdout/stderr contract tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/cli.rs src/config.rs src/error.rs src/output src/main.rs tests/cli.rs tests/output.rs
git commit -m "feat(cli): add text and JSON command interface"
```

### Task 6: End-to-end fixture coverage, CI, and release-facing documentation

**Files:**
- Create: `tests/end_to_end.rs`, `tests/live_cloudflare.rs`, `.github/workflows/ci.yml`
- Modify: `README.md`, `docs/MEASUREMENT_COMPATIBILITY.md`, `docs/TEST_STRATEGY.md`

**Interfaces:**
- Extends the local fixture from Task 3 to exercise a compact test plan through `Runner`; it does not add a public endpoint override.
- Adds ignored live tests for zero-byte latency, exact-size download, and upload broad invariants only.

- [ ] **Step 1: Add failing end-to-end tests**

```rust
#[tokio::test]
async fn compact_fixture_run_produces_reducible_results() {
    let server = FixtureServer::cloudflare_compatible().await;
    let outcome = run_fixture_plan(server.transport(), compact_plan()).await;
    assert!(outcome.error.is_none());
    assert!(outcome.result.summary.unloaded_latency_ms.is_some());
    assert!(outcome.result.summary.download_bps.is_some());
    assert!(outcome.result.summary.upload_bps.is_some());
    assert_eq!(server.unexpected_requests(), 0);
}

#[tokio::test]
#[ignore = "consumes external network resources"]
async fn live_cloudflare_zero_byte_probe_is_finite() {
    let observation = live_transport().latency(&CancellationToken::new()).await.unwrap();
    assert!(observation.ttfb.as_secs_f64().is_finite());
}
```

- [ ] **Step 2: Run the local end-to-end test and verify RED**

Run: `cargo test --test end_to_end`

Expected: fixture-plan helpers are missing or the compact run exposes an integration gap.

- [ ] **Step 3: Complete integration wiring and documentation**

Make the smallest changes needed for the local end-to-end path. Replace the documentation-only README with installation, usage, flags, text/JSON examples generated from the actual renderer, architecture summary, data-use warning, unofficial notice, native/browser timing disclosure, and upstream commit. Add CI jobs for Ubuntu, macOS, and Windows; Ubuntu runs fmt and Clippy in addition to tests and release build.

- [ ] **Step 4: Run all required gates**

Run:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
cargo build --release
```

Expected: every command exits `0`; live tests remain ignored.

- [ ] **Step 5: Review scope and commit**

Inspect `git diff --check`, `git status --short`, and the complete branch diff for retries, body concatenation, payload-sized upload allocation, stdout contamination, detached tasks, unsupported features, and unsupported parity claims.

```bash
git add README.md docs .github tests/end_to_end.rs tests/live_cloudflare.rs src Cargo.toml Cargo.lock
git commit -m "test(mvp): add integration coverage and CI"
```
