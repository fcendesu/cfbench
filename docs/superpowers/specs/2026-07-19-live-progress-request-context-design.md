# Live Progress and Cloudflare Request Context Design

- Status: Approved design; implementation pending
- Date: 2026-07-19
- Product: `cfbench`
- Compatibility impact: additive stderr progress; corrected production headers

## Purpose

Make ordinary text runs visibly report every individual request as it completes,
while preserving the existing no-TUI and machine-output contracts. Correct the
Cloudflare request context that caused large live downloads to return HTTP 403.

This work has two related outcomes:

1. users can see latency, bandwidth, and loaded-latency points during the run;
2. production requests match the same-origin browser context expected by
   Cloudflare's speed endpoints.

## Observed live failure and root cause

On 2026-07-19 a default native run completed exactly 169,000,000 bytes of
download payload, then failed at the first 100,000,000-byte download with:

```text
error: transport failed during download: endpoint
https://speed.cloudflare.com/__down returned HTTP status 403
```

The 169 MB total proves all download groups through 25 MB completed:

```text
100 KB initial + 9x100 KB + 8x1 MB + 6x10 MB + 4x25 MB = 169 MB
```

A headers-only reproduction against the 100 MB endpoint established the
request-context difference:

```text
no Origin or Referer: 403
Origin only:           200
Referer only:          200
Origin and Referer:    200
```

The probe used a client-side maximum-file-size guard and did not consume the
100 MB response. User-Agent changes were not required for the 200 response.

Cloudflare's browser engine runs on `https://speed.cloudflare.com/`, so normal
same-origin browser requests carry referrer/origin context that the native
client currently omits. The fix is to reproduce that context explicitly, not
to retry rejected measurements.

## Request headers

For production and compatible fixture base URLs:

- every latency and download `GET` sends `Referer: <base-url>/`;
- every upload `POST` sends `Referer: <base-url>/` and
  `Origin: <base-url-origin>`;
- post-plan metadata `GET /meta` sends the same Referer after the metadata
  feature is implemented;
- `Accept-Encoding: identity`, content headers, no-retry behavior, and all
  existing timing boundaries remain unchanged.

`Origin` contains only scheme and authority and never includes credentials,
path, query, or fragment. `Referer` is the normalized base URL with a trailing
slash and no credentials, query, or fragment. Transport construction returns a
typed setup error if either header cannot be represented safely.

Do not add a browser User-Agent merely to bypass server policy. The live
reproduction showed that request context, not User-Agent impersonation, was the
material difference.

No request is retried after HTTP 403, 429, or any other failure.

## Progress behavior

Progress is ordinary UTF-8 line output written to stderr. It never uses an
alternate screen, cursor movement, carriage-return rewriting, ANSI animation,
or a TUI dependency.

The existing opening line remains:

```text
Testing against Cloudflare edge...
```

Emit one result line for every completed successful individual request:

```text
[latency 1/20] 22.80 ms
[download 100 KB 1/9] 91.42 Mbps — 11.0 ms
[loaded/download 1] 25.40 ms
[upload 1 MB 1/6] 328.09 Mbps — 24.5 ms
[loaded/upload 1] 26.60 ms
```

Emit one immediate failure line when an individual request fails:

```text
[download 100 MB 1/3] failed — HTTP 403
```

The final text summary and terminal `error:` line remain authoritative. A
failure progress line is informational and must not replace the typed error,
partial-result behavior, or nonzero process status.

### Counters and labels

- unloaded latency uses phase-local `current/total` counters;
- download and upload use payload-group-local `current/total` counters;
- loaded latency uses a direction-local monotonically increasing counter;
- decimal payload labels are `100 KB`, `1 MB`, `10 MB`, `25 MB`, `50 MB`,
  `100 MB`, and `250 MB`;
- bandwidth is decimal Mbps with two fractional digits;
- latency is milliseconds with two fractional digits;
- transfer duration is adjusted measurement duration in milliseconds below
  one second and seconds at or above one second;
- failure details contain the stage and safe status/category only; credentials,
  URL queries, and response bodies are never printed.

The initial one-packet latency phase and later 20-packet replacement phase each
start their counter at one. The same is true for the bypassed initial 100 KB
download and the later 9-request 100 KB group; their group totals remain `1`
and `9`, respectively.

### Concurrent loaded latency

Loaded-latency lines are emitted when probes finish and may interleave with
download/upload lines. Their order reflects completion order. Because progress
reports every request, it may include a converted probe that is later excluded
from the summary when its transfer group fails the 250 ms loaded-request
eligibility rule, and it may show more than the latest 20 probes retained in the
raw result. Progress rendering must not acquire a lock across `.await` and must
not block or delay the next network measurement on terminal I/O.

Use a bounded progress channel between runner tasks and the stderr renderer.
When the channel is full, drop progress events rather than block measurement
timing. Dropped progress does not drop result points. After the runner and all
loaded probe tasks finish, close and join the progress-rendering task before
writing the final summary.

Use a capacity of 256 events. The default plan cannot normally fill this when
stderr keeps up, while the bound prevents unbounded memory use on a stalled
consumer.

### Adaptive stopping and unsupported stages

When early stopping prevents later payload groups, emit one direction-level
line:

```text
[download] larger payload groups skipped — request duration threshold reached
```

Emit it once per direction, immediately when the runner first skips a scheduled
group. Do not emit one line per skipped group.

The unsupported packet-loss plan entry emits:

```text
[packet loss] unavailable — TURN not configured
```

This is a stage status, not a fabricated measurement point.

## Output-mode contract

- ordinary text mode shows progress by default;
- `--quiet` suppresses every progress line, including the opening line,
  request results, failures, skip notices, and packet-loss notice;
- `--json` suppresses every progress line, preserving the current behavior of
  one JSON document on stdout and diagnostics/errors only on stderr;
- final text or JSON results are never suppressed by `--quiet`;
- fatal and nonfatal diagnostics remain visible under `--quiet`;
- redirected nonterminal stderr follows the same line-oriented behavior; no
  terminal capability detection is required.

## Architecture

Add a reqwest-independent progress event model at the orchestration boundary:

```rust
pub enum ProgressEvent {
    LatencyCompleted { current: u16, total: u16, latency_ms: f64 },
    TransferCompleted {
        direction: Direction,
        requested_bytes: u64,
        current: u16,
        total: u16,
        bps: u64,
        adjusted_duration_ms: f64,
    },
    LoadedLatencyCompleted {
        direction: Direction,
        sequence: u64,
        latency_ms: f64,
    },
    RequestFailed {
        stage: ProgressStage,
        current: Option<u16>,
        total: Option<u16>,
        kind: ProgressFailureKind,
    },
    DirectionFinished { direction: Direction },
    PacketLossUnavailable,
}
```

Exact Rust names may be refined in the implementation plan, but the public
behavior and reqwest-independent field set are binding. Events contain finite,
validated measurement values. Transfer and unloaded-latency success events
correspond to stored raw points; loaded-probe progress follows every converted
request even when later eligibility or latest-20 retention excludes it from the
result. `ProgressFailureKind` carries safe categories such as HTTP status,
timeout, cancellation, body stream, payload mismatch, and invalid measurement;
it does not wrap or expose reqwest errors.

Responsibilities remain separated:

- `transport` constructs browser-compatible request headers and returns typed
  observations/errors;
- `runner` emits domain progress events after point acceptance or terminal
  latency/transfer failure and owns counters/early-stop notices;
- `measurement/loaded_latency` emits successful and nonterminal failed probe
  events through the same bounded sender while preserving probe diagnostics;
- `output` formats progress events into stable ordinary lines;
- `app` owns the bounded channel, renderer task, stderr sink, suppression
  policy, and renderer joining;
- statistics and serialized result types do not depend on progress events.

Progress must not change public JSON point fields, reductions, request order,
payload sizes, timeouts, cancellation, or usage accounting.

## Error handling

- A closed progress receiver is equivalent to progress suppression; the runner
  continues normally.
- A stderr write failure is retained as an output error, triggers cancellation,
  and still joins the runner and probe tasks before returning.
- Progress formatting rejects non-finite numeric values and emits a safe
  unavailable category rather than serializing `NaN` or infinity.
- Terminal latency and transfer failure progress is emitted once at the runner
  boundary that records the failure and converts it into `RunnerError`; lower
  layers do not duplicate it.
- Loaded-probe transport failures and conversion rejections remain nonterminal:
  they create no `RunnerError` and emit exactly once inside
  `measurement/loaded_latency` while retaining the existing diagnostic.
- Cancellation progress is emitted only when cancellation selects before the
  request completes. A loaded probe cancelled by normal group shutdown is
  silent and does not record a diagnostic or failure event.

## Testing

Use TDD and local fixtures. Ordinary tests must not depend on Cloudflare.

Required coverage:

1. Download and latency fixture requests contain normalized `Referer`.
2. Upload fixture requests contain normalized `Referer` and `Origin`.
3. Request headers contain no credentials, query, or fragment.
4. No transport code contains or enables automatic retry behavior.
5. Every accepted unloaded-latency point emits one correctly numbered event.
6. Initial and later 100 KB groups use independent `1/1` and `1/9` counters.
7. Every accepted download/upload point emits payload, group counter, bps, and
   adjusted duration matching the stored point.
8. Loaded probe events may interleave, use monotonic per-direction sequence
   numbers, and report every converted probe independently of latest-20 result
   retention.
9. A transport failure emits one safe failure event and preserves the existing
   terminal error/partial result.
10. Adaptive finish emits one notice per finished direction.
11. Packet loss emits one unavailable notice and no point.
12. A full progress channel does not block or alter runner results.
13. Dropping the progress receiver does not fail the run.
14. Text mode writes events to stderr, while `--quiet` and `--json` write none.
15. JSON stdout remains exactly one parseable document.
16. Renderer write failure cancels and joins all active tasks.
17. Progress formatting produces the exact documented decimal units and
    punctuation without ANSI escapes or carriage returns.

Add an ignored live request-context test that requests the 100 MB endpoint with
a client-side body cancellation immediately after successful headers. It must
assert only that Cloudflare accepts the browser-compatible request context and
must not consume or buffer the full response.

## Documentation changes

Update in the implementation change:

- `README.md` live progress example, output streams, quiet/JSON behavior, and
  Cloudflare request-context disclosure;
- `docs/PRD.md` progress requirements and new exact examples;
- `docs/MVP.md` progress and live large-request acceptance criteria;
- `docs/MEASUREMENT_COMPATIBILITY.md` same-origin request headers without
  changing timing or retry semantics;
- `docs/TEST_STRATEGY.md` progress-channel and ignored live header coverage.

## Non-goals

- a TUI, spinner, progress bar, chart, or in-place line updates;
- per-network-chunk output;
- retrying 403, 429, timeout, or stream failures;
- browser User-Agent impersonation;
- changing Cloudflare payload sizes or adaptive stopping;
- packet loss implementation;
- making progress part of the JSON schema.

## Acceptance criteria

The feature is complete when text mode reports every accepted individual
request, progress suppression remains exact, progress cannot perturb or block
measurements, the 100 MB request-context regression passes, existing result and
error contracts remain unchanged, documentation matches behavior, and all
formatting, Clippy, test, and release-build gates pass.
