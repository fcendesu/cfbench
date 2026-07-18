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

Requirements:

- Set `Accept-Encoding: identity` where possible to avoid content encoding changing measured bytes.
- Consume download bodies completely unless cancelled or timed out.
- Consume upload responses completely even when the response body is small.
- Reuse a single configured client and connection pool for the run.
- Do not add retries around a measurement point. A retry is a new measurement and must be explicitly scheduled if ever supported.

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

### 5.2 Native request timestamps

Each request records:

- `request_started`: immediately before reqwest begins sending the request;
- `response_headers_received`: when `send().await` returns a response;
- `response_body_complete`: when the response body stream ends;
- `server_time_ms`: parsed from the response `Server-Timing` header, otherwise 10 ms;
- `payload_bytes`: actual body bytes read or intentionally uploaded.

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

- accept multiple comma-separated metrics;
- accept optional whitespace;
- read a duration expressed with `dur=<number>`;
- use the metric intended by Cloudflare when recognizable;
- ignore malformed and non-finite values;
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
- Treat the configured payload count as uploaded payload bytes.

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
- Probes begin while an enabled bandwidth request is active.
- The interval between probe starts is at least 400 ms.
- Loaded-latency points are retained for a payload-size group only when every completed transfer in that group lasts at least 250 ms.
- Keep no more than the latest 20 points for download and the latest 20 for upload.
- A cancelled or completed transfer must terminate its probe task promptly.
- Loaded probes and transfer body processing must not block one another on a synchronous mutex.

Upstream starts the loaded-latency engine after a 20 ms delay. The first probe starts then; subsequent probe starts are throttled by at least 400 ms. A transfer that ends before the delayed probe starts contributes no loaded-latency point.

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

## 11. Network and protocol behavior

### Client reuse

Use one `reqwest::Client` per test configuration so DNS, TLS, and pooled connections behave predictably.

### Warm-up

The upstream schedule already begins with latency and a bypassed 100 KB download estimate. Do not add an unreported warm-up request by default because it would change the measurement sequence.

### IPv4 and IPv6

- Auto mode uses normal resolution and connection behavior.
- IPv4-only and IPv6-only modes must prevent fallback to the other family.
- The implementation may need a custom resolver or connector if `local_address` alone cannot guarantee family selection on every platform.
- Integration tests should verify the connector behavior; do not infer the family only from a CLI flag.

### HTTP versions

Record the negotiated HTTP version for each point when available. A run may include more than one version after connection failures or re-establishment, so summary metadata may be `mixed`.

## 12. Cache and compression

- Match upstream by using only the `bytes=<N>` query for ordinary measurements and adding `during=download` or `during=upload` for loaded probes; do not add a random cache-busting parameter.
- Never allow an intermediary cache hit to produce a valid bandwidth point with zero transferred payload.
- Disable transparent decompression for measurement requests.
- Validate that the received download body count matches the requested size. A mismatch invalidates the point.

## 13. Compatibility report

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
| Packet loss | Unsupported in MVP | Explicit null status |
| HTTP/3 | Not required in MVP | Documented |
| AIM scores | Unsupported in MVP | Explicit absence |

## 14. Release validation

Before publishing `0.1.0`:

1. Compare the upstream main branch and latest release with this schedule and constants.
2. Confirm the upstream percentile implementation and port it exactly.
3. Confirm which `Server-Timing` metric is selected.
4. Confirm cache-busting behavior.
5. Confirm whether loaded-latency probing begins immediately or after one throttle interval.
6. Run at least 20 paired native/browser tests across slow, medium, and fast connections when available.
7. Record expected variance without defining browser output as an absolute oracle.

## 15. Primary references

- <https://github.com/cloudflare/speedtest>
- <https://github.com/cloudflare/speedtest/blob/main/README.md>
- <https://developer.mozilla.org/en-US/docs/Web/API/PerformanceResourceTiming>
- <https://docs.rs/reqwest/latest/reqwest/>

The `v1.11.0` source inspection used for this document covered `src/config/defaultConfig.ts`, `src/utils/numbers.ts`, `src/Results/MeasurementCalculations.ts`, `src/engines/BandwidthEngine/BandwidthEngine.ts`, `src/engines/BandwidthEngine/ParallelLatency.ts`, and `src/index.ts` at the commit recorded above.
