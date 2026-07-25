# Result Metadata and Timestamps Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add default Cloudflare edge/network/public-IP metadata, a privacy opt-out, one run timestamp, and completion timestamps on every raw point without affecting measurements.

**Architecture:** A bounded post-plan `/meta` request uses the same corrected transport and family policy after timed measurements finish. A Runner-owned wall-clock/monotonic anchor stamps accepted points; serializable metadata remains independent of reqwest and failures enrich diagnostics rather than terminating speed results.

**Tech Stack:** Stable Rust 1.95, Tokio 1.53, reqwest 0.13.4, Serde/serde_json, humantime 2.4.0 (MSRV 1.60) for RFC3339 formatting.

## Global Constraints

- Fetch metadata once after the measurement plan, never before or concurrently.
- Exclude metadata traffic/time from payload usage and `usage.duration_ms`.
- Use the same configured client, strict family, timeout, cancellation, identity encoding, Referer, and no-retry policy.
- Bound the `/meta` response to 65,536 bytes.
- `--no-metadata` performs zero metadata I/O and serializes `metadata_status: "disabled"`, `metadata: null`.
- Metadata failure is nonfatal and uses `metadata_status: "unavailable"` plus one diagnostic.
- Wall-clock values never participate in reductions, early stopping, timeout, or cancellation.
- Keep schema version 1; all fields are additive.

---

### Task 1: Serializable metadata and clock model

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Create: `src/clock.rs`
- Create: `src/results/metadata.rs`
- Modify: `src/results/mod.rs`
- Modify: `src/results/summary.rs`
- Modify: `src/results/point.rs`
- Modify: `src/measurement/timing.rs`
- Modify: `src/measurement/loaded_latency.rs`
- Modify: `src/runner.rs`
- Modify: `src/lib.rs`
- Modify: `tests/result_schema.rs`
- Modify: `tests/reductions.rs`
- Modify: `tests/runner.rs`
- Modify: `tests/loaded_latency.rs`

**Interfaces:**
- Produces: `RunClock::start`, `RunClock::started_at`, `RunClock::now_unix_ms`; `MetadataStatus`, `NetworkMetadata`, `ClientLocation`, `EdgeLocation`; timestamped `latency_point` and `bandwidth_point` conversion signatures.
- Consumed by: Tasks 3 and 4.

- [ ] **Step 1: Write failing schema tests**

Assert the additive shape and timestamp types:

```rust
#[test]
fn result_serializes_network_metadata_and_point_timestamps() {
    let mut result = RunResult::empty();
    result.started_at = "2026-07-19T09:02:59.123Z".into();
    result.target.metadata_status = MetadataStatus::Available;
    result.target.metadata = Some(metadata_fixture());
    result.raw.latency.push(latency_point_fixture(1_784_451_779_123));
    let json = serde_json::to_value(result).unwrap();
    assert_eq!(json["target"]["metadata"]["edge"]["colo"], "IST");
    assert_eq!(json["target"]["metadata"]["asn"], 12735);
    assert_eq!(json["points"]["latency"][0]["measured_at_unix_ms"], 1_784_451_779_123_i64);
}
```

Also assert disabled/unavailable metadata serializes as `null` with distinct statuses.

- [ ] **Step 2: Verify RED**

Run: `cargo test --test result_schema result_serializes_network_metadata_and_point_timestamps -- --exact`

Expected: FAIL because the result types and fields do not exist.

- [ ] **Step 3: Implement result types and monotonic wall-clock anchor**

Add `humantime = "2.4.0"`; its declared MSRV is Rust 1.60 and it formats `SystemTime` without unrelated runtime features. Implement:

```rust
#[derive(Clone)]
pub struct RunClock {
    started_instant: Instant,
    started_system: SystemTime,
    started_at: String,
}

impl RunClock {
    pub fn start() -> Self { /* capture adjacent Instant/SystemTime anchors */ }
    pub fn started_at(&self) -> &str { &self.started_at }
    pub fn now_unix_ms(&self) -> i64 { /* epoch anchor + monotonic elapsed */ }
}
```

Use signed `i64` epoch milliseconds with saturating conversion so pre-epoch/system-range edge cases never panic. Define metadata structs with nullable leaves and a lowercase serialized enum for `available`, `unavailable`, and `disabled`. Add `measured_at_unix_ms: i64` to latency/bandwidth points and `started_at: String` to `RunResult`.

Change conversion signatures so no fabricated timestamp can escape through the public measurement API:

```rust
pub fn latency_point(
    observation: TimingObservation,
    measured_at_unix_ms: i64,
) -> Result<LatencyPoint, MeasurementConversionError>;

pub fn bandwidth_point(
    direction: Direction,
    requested_bytes: u64,
    observation: TimingObservation,
    measured_at_unix_ms: i64,
) -> Result<BandwidthPoint, MeasurementConversionError>;
```

Update direct conversion fixtures to pass a fixed epoch value. Start one `RunClock` in Runner, assign `result.started_at`, pass `clock.now_unix_ms()` at every accepted unloaded/transfer conversion, and pass a cloned clock into loaded-probe conversion. This completes timestamp behavior in Task 1 while metadata enrichment remains isolated to Task 4.

- [ ] **Step 4: Verify GREEN and unchanged reductions**

Run: `cargo test --test result_schema --test reductions --test statistics --test runner --test loaded_latency`

Expected: PASS; changing timestamps does not change summaries.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/clock.rs src/results src/measurement/timing.rs src/measurement/loaded_latency.rs src/runner.rs src/lib.rs tests/result_schema.rs tests/reductions.rs tests/runner.rs tests/loaded_latency.rs
git commit -m "feat(results): add metadata and point timestamps"
```

---

### Task 2: CLI privacy policy

**Files:**
- Modify: `src/cli.rs`
- Modify: `src/config.rs`
- Modify: `tests/cli.rs`
- Modify: `tests/plan_compatibility.rs`

**Interfaces:**
- Produces: `Cli::no_metadata: bool`, `RunConfig::no_metadata: bool`.
- Consumed by: Runner/main in Task 4.

- [ ] **Step 1: Write failing CLI/config tests**

```rust
#[test]
fn no_metadata_is_public_and_defaults_to_collection() {
    let default = Cli::try_parse_from(["cfbench"]).unwrap();
    assert!(!RunConfig::try_from(default).unwrap().no_metadata);
    let disabled = Cli::try_parse_from(["cfbench", "--no-metadata"]).unwrap();
    assert!(RunConfig::try_from(disabled).unwrap().no_metadata);
}
```

Update the exact help-flag assertion to include `--no-metadata` with the approved disclosure wording.

- [ ] **Step 2: Verify RED**

Run: `cargo test --test cli --test plan_compatibility no_metadata -- --nocapture`

Expected: FAIL because the flag/config field does not exist.

- [ ] **Step 3: Add the flag and validated config field**

```rust
#[arg(
    long,
    help = "Skip the default public IP and network metadata request",
    long_help = "Metadata collection is enabled by default and includes the public IP, ASN, and approximate location already visible to Cloudflare. --no-metadata skips the request entirely."
)]
pub no_metadata: bool,
```

Map it directly in `TryFrom<Cli>` and default it to `false`.

- [ ] **Step 4: Verify GREEN and commit**

Run: `cargo test --test cli --test plan_compatibility`

```bash
git add src/cli.rs src/config.rs tests/cli.rs tests/plan_compatibility.rs
git commit -m "feat(cli): add metadata privacy opt-out"
```

---

### Task 3: Bounded Cloudflare metadata transport

**Files:**
- Create: `src/transport/metadata.rs`
- Modify: `src/transport/mod.rs`
- Modify: `src/transport/reqwest_transport.rs`
- Modify: `src/error.rs`
- Modify: `tests/support/mod.rs`
- Create: `tests/metadata.rs`

**Interfaces:**
- Produces: `ReqwestTransport::metadata(&CancellationToken) -> Result<NetworkMetadata, TransportError>` and tolerant `metadata_from_value(Value)` conversion.
- Consumed by: Runner metadata method in Task 4.

- [ ] **Step 1: Write failing parsing tests**

Use the observed Cloudflare `/meta` shape with IPv6 and an unknown field. Add cases where coordinates are strings, numbers, null, non-finite strings, and wrong types:

```rust
#[test]
fn maps_cloudflare_meta_and_rejects_only_invalid_leaves() {
    let metadata = metadata_from_value(json!({
        "clientIp": "2a02:ff0::1",
        "asn": 12735,
        "asOrganization": "TurkNet",
        "latitude": "41.01384",
        "longitude": {},
        "unknown": true,
        "colo": { "iata": "IST", "lat": 41.262222, "lon": "NaN", "cca2": "TR", "city": "Arnavutkoy" }
    })).unwrap();
    assert_eq!(metadata.public_ip.as_deref(), Some("2a02:ff0::1"));
    assert_eq!(metadata.client_location.latitude, Some(41.01384));
    assert_eq!(metadata.client_location.longitude, None);
    assert_eq!(metadata.edge.longitude, None);
}
```

- [ ] **Step 2: Verify parser RED, implement tolerant conversion, verify GREEN**

Run RED: `cargo test --test metadata maps_cloudflare_meta_and_rejects_only_invalid_leaves -- --exact`

Implement extraction from `serde_json::Value` rather than strict whole-object deserialization. Accept finite number/string coordinates and return null per invalid leaf. Run GREEN: `cargo test --test metadata maps_cloudflare_meta_and_rejects_only_invalid_leaves -- --exact`.

- [ ] **Step 3: Write failing bounded HTTP tests**

Extend the fixture with `/meta` success, malformed JSON, HTTP failure, delayed headers/body, and a 65,537-byte response. Assert Referer exists, response is read in chunks, 65,536 bytes succeeds, and 65,537 returns `MetadataBodyTooLarge` without retry.

- [ ] **Step 4: Implement bounded/cancellable metadata I/O**

Build `GET /meta` with the cached Referer and the same client/deadline/cancellation helpers. Stream chunks into a vector only while `len + chunk.len <= 65_536`, then parse with `serde_json::from_slice` and convert. Do not update payload usage or TimingObservation.

- [ ] **Step 5: Verify and commit**

Run: `cargo test --test metadata --test transport`

```bash
git add src/transport src/error.rs tests/support/mod.rs tests/metadata.rs
git commit -m "feat(transport): fetch bounded Cloudflare metadata"
```

---

### Task 4: Runner ordering and nonfatal metadata enrichment

**Files:**
- Modify: `src/runner.rs`
- Modify: `src/measurement/loaded_latency.rs`
- Modify: `src/main.rs`
- Modify: `tests/runner.rs`
- Modify: `tests/loaded_latency.rs`
- Modify: `tests/end_to_end.rs`

**Interfaces:**
- Consumes: timestamp-aware Runner from Task 1, result metadata, `ReqwestTransport::metadata`, `RunConfig::no_metadata`.
- Produces: metadata-aware Runner configuration with post-plan, nonfatal enrichment.

- [ ] **Step 1: Write failing timestamp/order tests**

Add a scripted transport that records operation order and returns metadata. Assert metadata occurs after the last plan request and usage duration is captured before metadata delay. Retain the Task 1 assertions that point timestamps are nondecreasing and loaded points carry timestamps.

```rust
assert_eq!(operations.last().map(String::as_str), Some("metadata"));
assert!(outcome.result.usage.duration_ms < metadata_delay_ms);
```

Add disabled, failure, and cancelled cases: disabled makes no call/status disabled; failure yields one diagnostic/status unavailable; cancellation skips metadata.

- [ ] **Step 2: Verify RED**

Run: `cargo test --test runner metadata -- --nocapture`

Expected: FAIL because Runner has no metadata operation or clock stamping.

- [ ] **Step 3: Extend transport abstraction and Runner**

Add a default metadata future on `MeasurementTransport` for scripted transports and override it for `ReqwestTransport`. Add `.with_metadata(bool)`. In `run_with_progress`:

1. execute the exact timestamp-aware plan from Task 1;
2. finalize usage duration and summary;
3. if enabled and not cancelled, request metadata once;
4. set available metadata or append one redacted diagnostic without replacing the measurement error.

- [ ] **Step 4: Wire CLI policy and verify GREEN**

In `main`, configure `.with_metadata(!config.no_metadata)`. Run:

`cargo test --test runner --test loaded_latency --test end_to_end`

Expected: PASS with unchanged measurement counts and post-plan metadata ordering.

- [ ] **Step 5: Commit**

```bash
git add src/runner.rs src/measurement/loaded_latency.rs src/main.rs tests/runner.rs tests/loaded_latency.rs tests/end_to_end.rs
git commit -m "feat(runner): enrich runs with metadata and timestamps"
```

---

### Task 5: Text/JSON presentation, live guard, docs, and gates

**Files:**
- Modify: `src/output/text.rs`
- Modify: `tests/output.rs`
- Modify: `tests/app.rs`
- Modify: `tests/live_cloudflare.rs`
- Modify: `README.md`
- Modify: `docs/PRD.md`
- Modify: `docs/MVP.md`
- Modify: `docs/MEASUREMENT_COMPATIBILITY.md`
- Modify: `docs/TEST_STRATEGY.md`

**Interfaces:**
- Produces: exact metadata text block and updated public documentation.

- [ ] **Step 1: Write failing text-output tests**

Cover complete, partial, unavailable, and disabled metadata:

```rust
assert!(text.contains("Edge (informational): IST — Arnavutkoy, TR"));
assert!(text.contains("Network: TurkNet Iletisim Hizmetleri A.S. (AS12735)"));
assert!(text.contains("Public IP: 2a02:ff0::1"));
assert!(text.contains("Measured at: 2026-07-19T09:02:59.123Z"));
assert!(!disabled_text.contains("Public IP:"));
assert!(unavailable_text.contains("Metadata: unavailable"));
```

- [ ] **Step 2: Verify RED, implement compact formatting, verify GREEN**

Run RED: `cargo test --test output metadata -- --nocapture`.

Implement punctuation-safe component joining and status-specific rendering. Run GREEN: `cargo test --test output --test app`.

- [ ] **Step 3: Add ignored live metadata coverage**

Add an ignored test that validates only nonempty public IP, ASN range, and colo shape without embedding returned values in assertion messages or committed fixtures.

- [ ] **Step 4: Update all contract documents**

Add `--no-metadata`, privacy disclosure, post-plan ordering, schema examples, per-point timestamps, raw grouping semantics, nonfatal failures, and test coverage. Preserve packet-loss/AIM non-goals.

- [ ] **Step 5: Run complete verification**

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
cargo build --release
git diff --check
```

Expected: all commands exit 0; ignored live/resource tests remain explicitly reported.

- [ ] **Step 6: Commit**

```bash
git add src/output/text.rs tests/output.rs tests/app.rs tests/live_cloudflare.rs README.md docs/PRD.md docs/MVP.md docs/MEASUREMENT_COMPATIBILITY.md docs/TEST_STRATEGY.md
git commit -m "docs(metadata): document result enrichment"
```
