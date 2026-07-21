# Measurement Compatibility Specification

- Product: `cfbench`
- Upstream baseline: Cloudflare Speedtest `v1.11.0`
- Upstream commit: `cfc99a74fd8d5c2121d319aeb7894c6246202c65`
- Baseline release date: 2026-07-01
- Document date: 2026-07-19

## 1. Purpose

This document is the implementation source of truth for matching Cloudflare Speedtest's published methodology. It prevents the project from calling a result “identical” when a native Rust client cannot observe the same browser timing fields.

Compatibility is divided into three levels:

- **Exact rule:** the same public configuration, sequence, threshold, or reduction can be implemented directly.
- **Native equivalent:** the same intent is implemented using native HTTP timing, but the observation point differs from a browser.
- **Unsupported in MVP:** the behavior requires a separate protocol or infrastructure and is represented honestly as unavailable.

## 2. Upstream endpoints

| Purpose | Method | Endpoint |
|---|---|---|
| Latency | GET | `https://speed.cloudflare.com/__down?bytes=0` |
| Download | GET | `https://speed.cloudflare.com/__down?bytes=<N>` |
| Upload | POST | `https://speed.cloudflare.com/__up` |
| Result metadata | GET | `https://speed.cloudflare.com/meta` |

Requirements:

- Set `Accept-Encoding: identity` where possible to avoid content encoding changing measured bytes.
- Send `Referer: https://speed.cloudflare.com/` on latency and download GETs.
- Send the same `Referer` plus `Origin: https://speed.cloudflare.com` on upload
  POSTs. Compatible fixture transports derive both values from their base URL.
- Normalize request context before timing: `Referer` is the base URL with a
  trailing slash; `Origin` contains only scheme and authority; neither includes
  credentials, query, or fragment.
- Consume download bodies completely unless cancelled or timed out.
- Consume upload responses completely even when the response body is small.
- Reuse a single configured client and connection pool for the run.
- Do not add retries around a measurement point. A retry is a new measurement and must be explicitly scheduled if ever supported.
- Fetch result metadata at most once after the timed measurement plan, never
  before or concurrently with a measurement. Use the same client, timeout,
  cancellation, strict address-family, redirect, proxy, decompression, Referer,
  and no-retry policy, and bound the response body to 65,536 bytes.

Reqwest `0.13.4` enables protocol-NACK retries by default (up to two retries in
addition to the original request). The production client builder must override
that behavior with `reqwest::retry::never()` so each scheduled measurement
operation makes at most one underlying transport attempt.

On 2026-07-19, a context-free 100 MB live GET returned HTTP 403 after the
preceding groups had transferred 169 MB. Headers-only probes showed 403 without
request context and 2xx with `Referer`, `Origin`, or both. Production GETs use
the same-origin `Referer` observed for the browser context; this correction does
not add a browser User-Agent, move the timing start, or retry the rejected
request.

## 3. Exact default schedule

```text
latency       packets=1
download      bytes=100000       count=1  bypass_finish=true
latency       packets=20
download      bytes=100000       count=9
download      bytes=1000000      count=8
upload        bytes=100000       count=8
packet_loss   packets=1000       responses_wait_ms=3000
upload        bytes=1000000      count=6
download      bytes=10000000     count=6
upload        bytes=10000000     count=4
download      bytes=25000000     count=4
upload        bytes=25000000     count=4
download      bytes=100000000    count=3
upload        bytes=50000000     count=3
download      bytes=250000000    count=2
```

The runner preserves this order. The packet-loss entry is skipped in the MVP but remains represented in plan metadata.

The optional `/meta` request is post-plan result enrichment, not a sixteenth
schedule entry. It starts only after every scheduled request and loaded probe
has stopped. It therefore cannot warm or compete with measurement traffic and
is excluded from plan payload accounting and duration. `--no-metadata` removes
the request entirely without changing this schedule.

The native baseline is encoded in `src/plan.rs` as a 15-entry compile-time
fixture with the upstream version and commit stored alongside it. Runtime
configuration derives a filtered plan from that fixture: disabling download or
upload removes only steps in that direction and preserves the relative order of
latency and unsupported packet-loss metadata steps. The compatibility fixture
in `tests/plan_compatibility.rs` compares every entry, including the initial
100 KB download's finish-gate bypass.

## 4. Configuration constants

| Setting | Value | Compatibility |
|---|---:|---|
| Loaded-latency throttle | 400 ms | Exact rule |
| Bandwidth finish request duration | 1000 ms | Exact rule |
| Bandwidth abort duration | 0 / disabled | Exact rule |
| Estimated server time fallback | 10 ms | Exact rule |
| Latency percentile | 0.5 | Exact rule |
| Bandwidth percentile | 0.9 | Exact rule |
| Minimum bandwidth request duration | 10 ms | Exact rule |
| Minimum loaded request duration | 250 ms | Exact rule |
| Maximum retained loaded-latency points | 20 | Exact rule |

## 5. Native timing model

### 5.1 Monotonic clock

All elapsed measurements must use `std::time::Instant`. Wall-clock timestamps may be included as metadata but must never be used to calculate latency or bandwidth.

One adjacent `SystemTime`/`Instant` anchor is captured immediately before the
first plan step. `started_at` is the UTC RFC 3339 form of that wall-clock
anchor. Every accepted raw point receives `measured_at_unix_ms` from the same
wall-clock anchor plus monotonic elapsed time. This makes point timestamps
nondecreasing even if the system wall clock changes. Wall-clock metadata does
not affect latency, bandwidth, jitter, percentiles, eligibility, loaded-point
retention, finish state, timeouts, cancellation, or `usage.duration_ms`.

### 5.2 Native request timestamps

Each request records:

- `request_started`: immediately before reqwest begins sending the request;
- `response_headers_received`: when `send().await` returns a response;
- `response_body_complete`: when the response body stream ends;
- `server_time_ms`: parsed from the response `Server-Timing` header, otherwise 10 ms;
- `payload_bytes`: download body bytes actually received, or upload body bytes
  yielded to reqwest. Yielded upload bytes are the closest native observable
  boundary and do not prove remote acceptance.

The configured timeout is one absolute deadline spanning request send,
response headers, and every response-body chunk. It does not restart when a
header or chunk arrives. Failures retain partial payload accounting.

Derived values:

```text
ttfb_ms = response_headers_received - request_started
request_duration_ms = response_body_complete - request_started
adjusted_ping_ms = max(0, ttfb_ms - server_time_ms)
adjusted_transfer_duration_ms = max(epsilon, request_duration_ms - server_time_ms)
estimated_transfer_bytes = payload_bytes * 1.005
bps = estimated_transfer_bytes * 8 / adjusted_transfer_duration_seconds
```

This is a native equivalent, not an exact browser observation.

### 5.3 Why native and browser results differ

Cloudflare's JavaScript engine uses `PerformanceResourceTiming`, including `requestStart`, `responseStart`, `responseEnd`, `transferSize`, and server timing. A native reqwest client does not receive the same browser-managed timeline.

Important differences:

1. `reqwest::Response` becomes available after response headers, which is used as the native approximation of response start.
2. Browser `transferSize` includes response headers plus encoded body bytes. When that value is unavailable, upstream estimates transfer bytes as payload bytes plus 0.5% header overhead. The native client applies the same 1.005 multiplier for bandwidth while retaining the actual counted payload bytes separately in results.
3. Browser connection reuse, DNS cache, proxy behavior, and protocol negotiation may differ from the Rust process.
4. Browsers may negotiate HTTP/3. Stable reqwest usage should assume HTTP/1.1 or HTTP/2 unless an explicitly tested HTTP/3 implementation is adopted later.
5. Browser service workers and cache behavior do not exist in the native client.

For large transfers, response-header overhead is negligible; it can matter more in 100 KB points. The JSON result must therefore identify the timing model as `native_reqwest_v1`.

## 6. Server-Timing parsing

The parser must:

- combine every `Server-Timing` field value in wire order with commas, then
  inspect the combined string using the pinned upstream search semantics;
- select the first decimal token matching `(?:^|;)\s*dur=([0-9.]+)`, regardless of metric name;
- accept optional whitespace after the start or semicolon boundary;
- preserve the upstream decimal-prefix behavior, so `dur=1e3` captures `1`;
- reject malformed or non-finite captured values;
- fall back to 10 ms if no valid server duration is found;
- clamp adjusted durations at zero or a small positive epsilon to prevent negative latency and division by zero.

Parsing must be unit tested against realistic headers, missing headers, duplicate metrics, malformed values, and decimal durations.

## 7. Latency

### Request

```text
GET /__down?bytes=0
```

### Point

A latency point is `adjusted_ping_ms` from the native timing model.

### Reduction

- Unloaded latency: 50th-percentile reduction over unloaded points.
- Loaded latency: the same latency percentile over retained loaded points.
- Jitter: arithmetic mean of `abs(point[i] - point[i-1])` for consecutive points.
- Jitter is unavailable when fewer than two points exist.
- Jitter is also unavailable when any input point is non-finite; the
  calculation otherwise preserves measurement order.

The initial one-packet latency estimate is replaced when the later 20-packet latency phase begins. Public unloaded points and reductions therefore use the 20-packet set; the initial estimate is not retained in the final result.

## 8. Bandwidth

### Download

- Start timer immediately before sending the GET request.
- Read body chunks until EOF.
- Sum actual body bytes.
- Record headers-arrival and body-completion timestamps.
- Reject an HTTP error status before treating the point as valid.

### Upload

- Produce exactly the configured payload byte count.
- Use a reusable or streaming body source to prevent repeated payload-sized allocation.
- Start timer immediately before sending.
- Stop after the response body is complete.
- Count bytes as the bounded upload stream yields them to reqwest; a successful
  upload must yield the configured payload count exactly.
- A final 2xx response does not make a partially yielded request valid. After
  response EOF, snapshot the yielded count once; if it differs from the
  requested count, return an upload-specific payload-mismatch error, retain the
  partial usage, and do not create a bandwidth point.

### Inclusion and reduction

- A bandwidth point is eligible for the final summary when its adjusted request duration is at least 10 ms.
- Final direction bandwidth is the 90th percentile of eligible bps points.
- A group marks a direction finished only after all requests in that group complete and the minimum adjusted duration across the group is strictly greater than 1000 ms, unless that group has `bypass_finish=true`.
- Once a direction is finished, later groups in that direction are skipped; interleaved groups in the other direction continue.
- Percentiles use the pinned upstream sorted linear-interpolation algorithm at index `(len - 1) * percentile`; fixture tests must keep this behavior exact.
- Empty input, an out-of-range or non-finite percentile fraction, or any
  non-finite input value produces an unavailable reduction rather than a
  partial or non-finite result.
- A non-finite computed result, including intermediate overflow from finite
  extreme inputs, is also unavailable.

## 9. Loaded latency

- Loaded probes use the zero-byte download endpoint.
- One probe loop begins for each enabled payload-size group and remains active across every sequential request in that group.
- The interval between probe starts is at least 400 ms.
- Loaded-latency points are retained for a payload-size group only when every completed transfer in that group lasts at least 250 ms.
- Keep no more than the latest 20 points for download and the latest 20 for upload.
- A cancelled or completed transfer must terminate its probe task promptly.
- Loaded probes and transfer body processing must not block one another on a synchronous mutex.

Upstream starts the group-scoped loaded-latency engine after a 20 ms delay. The first probe starts then; subsequent probe starts are throttled by at least 400 ms. A group that ends before the delayed probe starts contributes no loaded-latency point.

## 10. Packet loss

Cloudflare's method sends UDP packets through a WebRTC TURN server and counts round-trip losses. The public TURN path is deprecated, and upstream requires user-provided TURN configuration.

MVP behavior:

```text
packet_loss.status = "unavailable"
packet_loss.reason = "turn_not_implemented"
packet_loss.ratio = null
```

The MVP must not perform ICMP ping loss and label it as Cloudflare packet loss.

Post-MVP implementation requires a separate design covering TURN credentials, UDP transport, batching, security, and cross-platform behavior.

## 11. Post-plan metadata and raw-result semantics

Metadata collection is enabled by default and may be disabled with
`--no-metadata`. The bounded `/meta` object supplies public client IP, unsigned
32-bit ASN, network organization, approximate client location, and edge
colo/location. All optional leaves are nullable, unknown upstream fields are
ignored, and invalid or non-finite coordinates become null individually.
Cloudflare is the only source; the MVP does not contact a third-party IP or ASN
service.

`target.metadata_status` has exact policy/error semantics:

- `available`: one bounded valid top-level `/meta` JSON object was accepted,
  even when optional leaves are null;
- `unavailable`: collection was enabled but the HTTP request, body limit, JSON,
  or top-level object validation failed;
- `disabled`: `--no-metadata` caused zero metadata I/O.

`target.metadata` is null for unavailable and disabled. Retrieval failure is
nonfatal: it appends one redacted diagnostic, preserves measurement points and
the measurement-derived process status, and does not fabricate latency or
bandwidth data. Cancellation during post-plan enrichment remains cancellation;
a run cancelled during measurements skips metadata. If active metadata is
cancelled after a terminal measurement error, cancellation supersedes that
error as the terminal outcome while the earlier failure and completed points
remain in the serialized history. The metadata cancellation is appended to
`failures`, not reduced to a diagnostic.

Raw JSON keeps one measurement-ordered array for public unloaded latency, one
array per bandwidth direction, and one latest-20 array per loaded-latency
direction. `requested_bytes` on a bandwidth point is the canonical payload-size
group key; points are not duplicated into a second grouped object. The initial
one-packet latency estimate remains private orchestration state and is not
serialized. Every successful serialized latency and bandwidth point carries
its completion timestamp.

Packet loss remains `unavailable`/null because TURN/WebRTC is not implemented,
and Cloudflare AIM/network-quality scores remain absent. `/meta` must not be
used to fabricate either excluded feature.

## 12. Network and protocol behavior

### Client reuse

Use one `reqwest::Client` per test configuration so DNS, TLS, and pooled connections behave predictably.

### Warm-up

The upstream schedule already begins with latency and a bypassed 100 KB download estimate. Do not add an unreported warm-up request by default because it would change the measurement sequence.

### IPv4 and IPv6

- Auto mode uses normal resolution and connection behavior.
- IPv4-only and IPv6-only modes must prevent fallback to the other family.
- Forced-family clients disable system proxies because proxy resolution and
  connection establishment cannot guarantee the requested target family.
  Auto mode retains reqwest's standard proxy behavior.
- The implementation may need a custom resolver or connector if `local_address` alone cannot guarantee family selection on every platform.
- Integration tests should verify the connector behavior; do not infer the family only from a CLI flag.

### HTTP versions

Record the negotiated HTTP version for each point when available. A run may include more than one version after connection failures or re-establishment, so summary metadata may be `mixed`.

### Progress reporting integrity

Ordinary text mode reports accepted requests and stage status through stable
stderr lines such as:

```text
[latency 1/20] 22.80 ms
[download 100 KB 1/9] 91.42 Mbps — 11.0 ms
[loaded/download 1] 25.40 ms
[upload 1 MB 1/6] 328.09 Mbps — 24.5 ms
```

`--quiet` and `--json` suppress all progress; diagnostics remain on stderr and
JSON stdout remains one document. Event sending uses bounded nonblocking
`try_send`, so slow output may drop a display event but cannot block a request,
drop a raw point, change usage, or affect reductions. Request builders and
headers are completed before monotonic measurement timing begins.

## 13. Cache and compression

- Match upstream by using only the `bytes=<N>` query for ordinary measurements and adding `during=download` or `during=upload` for loaded probes; do not add a random cache-busting parameter.
- Never allow an intermediary cache hit to produce a valid bandwidth point with zero transferred payload.
- Disable transparent decompression for measurement requests.
- Validate that the received download body count matches the requested size. A mismatch invalidates the point.

## 14. Compatibility report

Every release should maintain a table like this:

| Behavior | Status | Validation |
|---|---|---|
| Endpoints | Exact | Integration test |
| Default schedule | Exact | Static fixture |
| Thresholds | Exact | Unit tests |
| Percentile algorithm | Exact after source validation | Fixture parity test |
| Jitter | Exact rule | Unit tests |
| Server-time fallback | Exact rule | Header fixtures |
| Browser TTFB | Native equivalent | Side-by-side live runs |
| Browser transferSize | Native payload approximation | Documented |
| Loaded latency | Intended equivalent | Concurrency tests and live runs |
| `/meta` enrichment | Informational, outside plan | Bounded local fixtures and ignored live shape guard |
| Packet loss | Unsupported in MVP | Explicit null status |
| HTTP/3 | Not required in MVP | Documented |
| AIM scores | Unsupported in MVP | Explicit absence |

## 15. Release validation

Before publishing `0.1.0`:

1. Compare the upstream main branch and latest release with this schedule and constants.
2. Confirm the upstream percentile implementation and port it exactly.
3. Confirm which `Server-Timing` metric is selected.
4. Confirm cache-busting behavior.
5. Confirm whether loaded-latency probing begins immediately or after one throttle interval.
6. Run at least 20 paired native/browser tests across slow, medium, and fast connections when available.
7. Record expected variance without defining browser output as an absolute oracle.

Release integration coverage runs a compact immutable plan through `Runner`
and the real `ReqwestTransport` against a local Cloudflare-compatible fixture.
It covers unloaded latency plus reducible download and upload points without
adding a public endpoint override. The fixture rejects unknown request shapes,
so the test also detects unexpected network activity.

Ignored live tests use a zero-byte latency probe, a 65,536-byte download, and a
65,536-byte upload. A separate 100,000,000-byte request-context regression guard
waits only for successful response headers and drops the response without
reading its body. The tests assert endpoint and finite-timing invariants rather
than speed values. They are excluded from ordinary CI because public network
tests depend on external service behavior and consume resources.

A separate ignored `/meta` guard validates only that the public-IP field is
nonempty, the ASN is positive and within the `u32` result type, and the colo is
three uppercase ASCII letters. The test does not place returned public IP,
network, or location values in assertion messages, committed fixtures, or
successful test output.

## 16. Primary references

- <https://github.com/cloudflare/speedtest>
- <https://github.com/cloudflare/speedtest/blob/main/README.md>
- <https://developer.mozilla.org/en-US/docs/Web/API/PerformanceResourceTiming>
- <https://docs.rs/reqwest/latest/reqwest/>

The `v1.11.0` source inspection used for this document covered `src/config/defaultConfig.ts`, `src/utils/numbers.ts`, `src/Results/MeasurementCalculations.ts`, `src/engines/BandwidthEngine/BandwidthEngine.ts`, `src/engines/BandwidthEngine/ParallelLatency.ts`, and `src/index.ts` at the commit recorded above.
