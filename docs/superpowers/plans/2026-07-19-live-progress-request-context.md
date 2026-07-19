# Live Progress and Request Context Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Report every individual Cloudflare measurement request as line-oriented stderr progress and add the same-origin request headers required for large live downloads.

**Architecture:** `ReqwestTransport` constructs normalized Referer/Origin headers once and applies them without changing timing boundaries. A reqwest-independent `ProgressEvent` model flows through a bounded nonblocking channel to a dedicated blocking stderr renderer; Runner and loaded-probe tasks emit events without awaiting terminal I/O.

**Tech Stack:** Stable Rust 1.95, Tokio 1.53, reqwest 0.13.4, std bounded channels/threads, existing Clap/thiserror test stack.

## Global Constraints

- Preserve the exact Cloudflare v1.11.0 measurement plan, reductions, payload sizes, and early stopping.
- Never retry a measurement request.
- Never emit progress to stdout; `--quiet` and `--json` suppress all progress.
- Use no cursor movement, carriage-return rewriting, ANSI animation, TUI, or per-chunk output.
- Progress delivery uses `try_send`; a slow/closed renderer never blocks measurements or drops result points.
- Request context contains no credentials, query, or fragment.
- Complete every RED-GREEN cycle before changing unrelated production code.

---

### Task 1: Browser-compatible request context

**Files:**
- Modify: `src/transport/reqwest_transport.rs`
- Modify: `src/error.rs`
- Modify: `tests/support/mod.rs`
- Modify: `tests/transport.rs`

**Interfaces:**
- Consumes: `ReqwestTransport::with_base_url`, existing local fixture server.
- Produces: cached `referer: HeaderValue` and `origin: HeaderValue`; all downloads send Referer and all uploads send Referer plus Origin.

- [ ] **Step 1: Write failing integration tests**

Extend captured fixture requests with `method`, `path`, `referer`, and `origin`, then add tests with a base URL containing credentials/query/fragment:

```rust
#[tokio::test]
async fn measurement_requests_send_safe_same_origin_context() {
    let fixture = FixtureServer::cloudflare_compatible().await;
    let transport = ReqwestTransport::with_base_url(
        RunConfig::default(),
        fixture.url_with_test_context(),
    ).unwrap();
    let cancel = CancellationToken::new();

    transport.download(100_000, None, &cancel).await.unwrap();
    transport.upload(100_000, &cancel).await.unwrap();

    let requests = fixture.requests().await;
    assert_eq!(requests[0].referer.as_deref(), Some(&format!("{}/", fixture.url())));
    assert_eq!(requests[0].origin, None);
    assert_eq!(requests[1].referer, requests[0].referer);
    assert_eq!(requests[1].origin.as_deref(), Some(fixture.url().as_str()));
    assert!(!requests.iter().any(|r| format!("{r:?}").contains("secret")));
}
```

- [ ] **Step 2: Verify RED**

Run: `cargo test --test transport measurement_requests_send_safe_same_origin_context -- --exact`

Expected: FAIL because captured request context and transport headers do not exist.

- [ ] **Step 3: Implement normalized header construction**

Store two header values on `ReqwestTransport` and construct them before building requests:

```rust
fn request_context(base_url: &Url) -> Result<(HeaderValue, HeaderValue), TransportError> {
    let mut referer = base_url.clone();
    referer.set_username("").map_err(|_| TransportError::InvalidRequestContext)?;
    referer.set_password(None).map_err(|_| TransportError::InvalidRequestContext)?;
    referer.set_path("/");
    referer.set_query(None);
    referer.set_fragment(None);
    let origin = referer.origin().ascii_serialization();
    Ok((
        HeaderValue::from_str(referer.as_str()).map_err(|_| TransportError::InvalidRequestContext)?,
        HeaderValue::from_str(&origin).map_err(|_| TransportError::InvalidRequestContext)?,
    ))
}
```

Apply `REFERER` to GET/POST builders and `ORIGIN` only to uploads. Keep endpoint redaction and request timing start immediately before `send()`.

- [ ] **Step 4: Verify GREEN and header invariants**

Run: `cargo test --test transport measurement_requests_send_safe_same_origin_context transport_errors_include_redacted_endpoint_context -- --nocapture`

Expected: PASS with no credential/query leakage.

- [ ] **Step 5: Commit**

```bash
git add src/transport/reqwest_transport.rs src/error.rs tests/support/mod.rs tests/transport.rs
git commit -m "fix(transport): send Cloudflare request context"
```

---

### Task 2: Progress domain model and exact formatter

**Files:**
- Create: `src/progress.rs`
- Create: `src/output/progress.rs`
- Modify: `src/lib.rs`
- Modify: `src/output/mod.rs`
- Create: `tests/progress.rs`

**Interfaces:**
- Produces: `ProgressEvent`, `ProgressFailureKind`, `ProgressReporter::channel`, `ProgressReporter::disabled`, `ProgressReporter::emit`, and `output::render_progress`.
- Consumed by: Tasks 3 and 4.

- [ ] **Step 1: Write formatter and backpressure tests**

```rust
#[test]
fn formats_individual_transfer_progress_without_terminal_control() {
    let line = render_progress(&ProgressEvent::TransferCompleted {
        direction: Direction::Download,
        requested_bytes: 100_000_000,
        current: 1,
        total: 3,
        bps: 676_870_000,
        adjusted_duration_ms: 1_188.4,
    });
    assert_eq!(line, "[download 100 MB 1/3] 676.87 Mbps — 1.19 s");
    assert!(!line.contains(['\r', '\u{1b}']));
}

#[test]
fn full_or_closed_progress_channel_never_blocks_or_fails() {
    let (reporter, receiver) = ProgressReporter::channel(1);
    reporter.emit(ProgressEvent::PacketLossUnavailable);
    reporter.emit(ProgressEvent::PacketLossUnavailable);
    drop(receiver);
    reporter.emit(ProgressEvent::PacketLossUnavailable);
}
```

- [ ] **Step 2: Verify RED**

Run: `cargo test --test progress -- --nocapture`

Expected: FAIL because the progress model and formatter do not exist.

- [ ] **Step 3: Implement the bounded reporter and domain events**

Use `std::sync::mpsc::sync_channel` and `try_send`:

```rust
#[derive(Clone)]
pub struct ProgressReporter(Option<SyncSender<ProgressEvent>>);

impl ProgressReporter {
    pub fn channel(capacity: usize) -> (Self, Receiver<ProgressEvent>) {
        let (sender, receiver) = sync_channel(capacity);
        (Self(Some(sender)), receiver)
    }

    pub fn disabled() -> Self { Self(None) }

    pub fn emit(&self, event: ProgressEvent) {
        if let Some(sender) = &self.0 {
            let _ = sender.try_send(event);
        }
    }
}
```

Define the event variants from the approved spec. Implement decimal payload labels and finite-safe latency/bandwidth/duration formatting in `src/output/progress.rs`.

- [ ] **Step 4: Verify GREEN**

Run: `cargo test --test progress`

Expected: PASS for exact lines, no ANSI/CR, full channel, and closed channel.

- [ ] **Step 5: Commit**

```bash
git add src/progress.rs src/output/progress.rs src/lib.rs src/output/mod.rs tests/progress.rs
git commit -m "feat(progress): add bounded progress events"
```

---

### Task 3: Runner and loaded-probe event emission

**Files:**
- Modify: `src/runner.rs`
- Modify: `src/measurement/loaded_latency.rs`
- Modify: `tests/runner.rs`
- Modify: `tests/loaded_latency.rs`

**Interfaces:**
- Consumes: `ProgressReporter` and events from Task 2.
- Produces: `Runner::run_with_progress(&CancellationToken, ProgressReporter)`; existing `run` delegates with a disabled reporter.

- [ ] **Step 1: Write failing Runner event tests**

Add a collector channel and assert the exact event sequence for a compact scripted plan:

```rust
let (progress, receiver) = ProgressReporter::channel(256);
let outcome = runner.run_with_progress(&cancel, progress).await;
let events: Vec<_> = receiver.into_iter().collect();
assert!(matches!(events[0], ProgressEvent::LatencyCompleted { current: 1, total: 1, .. }));
assert!(events.iter().any(|event| matches!(event,
    ProgressEvent::TransferCompleted { requested_bytes: 100_000, current: 1, total: 9, .. }
)));
assert_eq!(outcome.result.raw.download.len(), expected_download_points);
```

Add separate RED tests for one safe failure event, one direction-finished event, and one packet-loss-unavailable event.

- [ ] **Step 2: Verify RED**

Run: `cargo test --test runner progress -- --nocapture`

Expected: FAIL because `run_with_progress` and Runner event emission do not exist.

- [ ] **Step 3: Implement sequential event emission**

Pass `&ProgressReporter` through `execute`, latency phases, and transfer groups. Use `enumerate()` to derive phase/group counters. Emit success only after point conversion succeeds and emit failure exactly where `RunnerError` is recorded.

Keep `run` backward compatible:

```rust
pub async fn run(&self, cancellation: &CancellationToken) -> RunOutcome {
    self.run_with_progress(cancellation, ProgressReporter::disabled()).await
}
```

Track one boolean per direction so `DirectionFinished` is emitted once when the runner first encounters a later skipped group.

- [ ] **Step 4: Add loaded-probe reporting RED/GREEN**

Write a failing loaded-latency test, then extend `spawn_loaded_probe_loop` with a reporter and shared direction-local `AtomicU64`. Emit `LoadedLatencyCompleted` immediately after successful conversion, before latest-20 retention/eligibility decisions. Confirm a short ineligible group can report a probe without adding it to raw results.

Run: `cargo test --test loaded_latency --test runner`

Expected: PASS with no detached probe tasks and monotonic direction-local sequences.

- [ ] **Step 5: Commit**

```bash
git add src/runner.rs src/measurement/loaded_latency.rs tests/runner.rs tests/loaded_latency.rs
git commit -m "feat(runner): emit individual measurement progress"
```

---

### Task 4: Live stderr renderer lifecycle

**Files:**
- Modify: `src/app.rs`
- Modify: `src/main.rs`
- Modify: `tests/app.rs`
- Modify: `tests/cli.rs`

**Interfaces:**
- Consumes: Runner progress channel and formatter.
- Produces: `spawn_progress_renderer`, `run_with_signal_and_progress`; joined renderer lifecycle with cancellation on write failure.

- [ ] **Step 1: Write failing output-mode and lifecycle tests**

Cover ordinary text, quiet, JSON, writer failure, and channel closure:

```rust
#[tokio::test]
async fn text_mode_streams_progress_but_quiet_and_json_do_not() {
    let text = run_fixture(OutputOptions { json: false, quiet: false }).await;
    assert!(text.stderr.contains("[latency 1/1]"));

    let quiet = run_fixture(OutputOptions { json: false, quiet: true }).await;
    assert!(!quiet.stderr.contains("[latency"));

    let json = run_fixture(OutputOptions { json: true, quiet: false }).await;
    assert!(!json.stderr.contains("[latency"));
    serde_json::from_str::<Value>(&json.stdout).unwrap();
}
```

- [ ] **Step 2: Verify RED**

Run: `cargo test --test app text_mode_streams_progress_but_quiet_and_json_do_not -- --exact`

Expected: FAIL because only the opening progress line exists.

- [ ] **Step 3: Implement renderer ownership and joining**

Create a 256-capacity reporter only for ordinary non-quiet text mode. Move the receiver and owned writer into a dedicated thread that calls blocking `recv`, formats one line, writes/flushed stderr, and cancels the shared token on write failure. Pass the reporter by value into `Runner::run_with_progress`; after Runner and all probe clones drop, join the renderer before final output.

Retain `run_with_signal` for no-progress library tests and factor cancellation selection into one internal helper so signal behavior is not duplicated.

- [ ] **Step 4: Verify GREEN and all app contracts**

Run: `cargo test --test app --test cli --test progress`

Expected: PASS; JSON stdout is one document and renderer failure joins/cancels the run.

- [ ] **Step 5: Commit**

```bash
git add src/app.rs src/main.rs tests/app.rs tests/cli.rs
git commit -m "feat(cli): stream individual request progress"
```

---

### Task 5: Live guard, documentation, and verification

**Files:**
- Modify: `tests/live_cloudflare.rs`
- Modify: `README.md`
- Modify: `docs/PRD.md`
- Modify: `docs/MVP.md`
- Modify: `docs/MEASUREMENT_COMPATIBILITY.md`
- Modify: `docs/TEST_STRATEGY.md`

**Interfaces:**
- Produces: ignored header-acceptance guard and release-facing progress/request-context documentation.

- [ ] **Step 1: Write the ignored live header test**

Add an ignored test that sends the 100 MB GET with normalized Referer, validates 2xx headers, and drops the response before reading the body:

```rust
#[tokio::test]
#[ignore = "uses the live Cloudflare endpoint"]
async fn live_large_download_accepts_browser_request_context() {
    let transport = ReqwestTransport::new(RunConfig::default()).unwrap();
    let status = transport.probe_download_headers(100_000_000).await.unwrap();
    assert!(status.is_success());
}
```

The helper is test-only or crate-private and must reuse production header construction/no-retry policy.

- [ ] **Step 2: Update all public documents**

Document exact progress examples, stderr suppression, same-origin Referer/Origin behavior, no retries, and the live 403 regression. Do not claim the ignored live test was executed unless it is run in this session.

- [ ] **Step 3: Run focused and full quality gates**

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
cargo build --release
git diff --check
```

Expected: all commands exit 0; live tests remain ignored unless explicitly invoked.

- [ ] **Step 4: Review measurement integrity**

Confirm from the diff that event sending uses nonblocking `try_send`, no progress reaches stdout, no request retry was introduced, request timing starts after builder/header setup, and result reductions/usage are unchanged.

- [ ] **Step 5: Commit**

```bash
git add tests/live_cloudflare.rs README.md docs/PRD.md docs/MVP.md docs/MEASUREMENT_COMPATIBILITY.md docs/TEST_STRATEGY.md
git commit -m "docs(progress): document live request reporting"
```
