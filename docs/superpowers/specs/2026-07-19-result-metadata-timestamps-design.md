# Result Metadata and Timestamps Design

- Status: Approved design; implementation pending
- Date: 2026-07-19
- Product: `cfbench`
- Compatibility impact: additive CLI and JSON fields

## Purpose

Add the non-packet-loss data needed to describe and reproduce a Cloudflare
speed-test run without adding a TUI or changing the measurement algorithm:

- Cloudflare edge colo and location;
- public client IP address;
- client ASN and network organization;
- one wall-clock run timestamp;
- one wall-clock completion timestamp per successful raw point.

Raw latency points and raw bandwidth points already exist in the JSON result.
Bandwidth points already carry `requested_bytes`, which is the canonical group
key for the Cloudflare payload-size groups. This feature must not duplicate the
same points into a second grouped structure.

## Source of network metadata

Use one `GET https://speed.cloudflare.com/meta` request. On 2026-07-19 the
Cloudflare endpoint returned a JSON object with this public shape:

```json
{
  "hostname": "speed.cloudflare.com",
  "clientIp": "2a02:ff0:...",
  "httpProtocol": "HTTP/1.1",
  "asn": 12735,
  "asOrganization": "TurkNet Iletisim Hizmetleri A.S.",
  "country": "TR",
  "city": "Istanbul",
  "region": "Istanbul",
  "postalCode": "34096",
  "latitude": "41.01384",
  "longitude": "28.94966",
  "colo": {
    "iata": "IST",
    "lat": 41.262222,
    "lon": 28.727778,
    "cca2": "TR",
    "region": "Europe",
    "city": "Arnavutkoy"
  }
}
```

The implementation consumes only the fields included in the public result
model below. Unknown response fields must be ignored so additive upstream
changes do not break a run. Missing or `null` optional fields remain `None`.

Do not call a third-party IP intelligence service. The ASN, organization,
client location, and edge location must all come from Cloudflare's `/meta`
response.

## Request ordering and measurement integrity

Metadata is enabled by default. Fetch it once **after** the timed measurement
plan finishes, not before or concurrently with it. Using the same configured
reqwest client after the plan preserves these invariants:

- the metadata request cannot warm DNS, TLS, or the connection pool before the
  initial latency measurement;
- it cannot compete with bandwidth transfers or loaded-latency probes;
- it cannot alter the published Cloudflare measurement order;
- it follows the same strict IPv4-only or IPv6-only transport policy;
- it does not require a second production HTTP client.

The metadata request is excluded from `usage.download_payload_bytes`,
`usage.upload_payload_bytes`, and `usage.duration_ms`. `usage.duration_ms`
continues to represent only the speed-test plan. A cancelled run skips the
metadata request. A non-cancellation terminal measurement failure may still be
enriched after all measurement tasks have stopped.

The metadata request must use the existing absolute request timeout,
cancellation, redirect, proxy, decompression, and no-retry policy. Its response
body must be bounded to 64 KiB before JSON parsing. Metadata endpoint URLs in
errors and diagnostics use the existing credential/query redaction rules.

## CLI behavior and privacy

Add one flag:

```text
--no-metadata    Do not request or display public IP and network metadata
```

Default behavior requests and displays metadata. `--no-metadata` skips the
network request entirely; it is not merely a rendering switch. In JSON mode,
`target.metadata` serializes as `null`. In text mode, metadata lines are
omitted rather than printed as `unavailable` when the flag is set.

`target.metadata_status` distinguishes `available`, `unavailable`, and
`disabled`; renderers must not infer policy from diagnostics or from a null
metadata object.

The README and `--help` must disclose that the default metadata request exposes
the public IP, ASN, and approximate location already visible to Cloudflare.

Metadata retrieval failure is nonfatal. The speed-test result and process exit
status remain governed by the measurement run. On metadata failure:

- `target.metadata` is `null`;
- text output displays `Metadata: unavailable` when metadata was enabled;
- one concise diagnostic is written to stderr;
- no failed bandwidth or latency point is fabricated.

## Result model

Keep `schema_version` at `1`. The result is pre-release and all changes are
additive: existing fields retain their names, types, and semantics. Unknown
fields are expected to be ignored by consumers.

Add `started_at` to the result envelope. It is an RFC 3339 UTC string captured
immediately before the first measurement step:

```json
{
  "schema_version": 1,
  "started_at": "2026-07-19T09:02:59.123Z"
}
```

Extend `target` with nullable metadata:

```json
{
  "target": {
    "provider": "cloudflare",
    "ip_family": "ipv6",
    "http_version": "2",
    "timing_model": "native_reqwest_v1",
    "metadata_status": "available",
    "metadata": {
      "public_ip": "2a02:ff0:...",
      "asn": 12735,
      "as_organization": "TurkNet Iletisim Hizmetleri A.S.",
      "client_location": {
        "country_code": "TR",
        "city": "Istanbul",
        "region": "Istanbul",
        "postal_code": "34096",
        "latitude": 41.01384,
        "longitude": 28.94966
      },
      "edge": {
        "colo": "IST",
        "country_code": "TR",
        "region": "Europe",
        "city": "Arnavutkoy",
        "latitude": 41.262222,
        "longitude": 28.727778
      }
    }
  }
}
```

All metadata leaves are nullable because Cloudflare may omit individual fields.
`target.metadata` is present only when a valid JSON object was received; it is
`null` when disabled or when retrieval/parsing fails. Public IP remains a
string and must support IPv4 and IPv6 without lossy normalization. ASN is an
unsigned 32-bit integer. Latitude and longitude must be finite before entering
the public result.

`metadata_status` has exactly these meanings:

- `available`: a bounded valid `/meta` JSON object was accepted, even if some
  optional leaves are null;
- `unavailable`: collection was enabled but the request or response failed;
- `disabled`: `--no-metadata` skipped the request.

Add `measured_at_unix_ms` to every successful serialized `LatencyPoint` and
`BandwidthPoint`. It is the Unix epoch millisecond at which cfbench completed
the observation and accepted it as a raw point:

```json
{
  "ping_ms": 21.6,
  "measured_at_unix_ms": 1784451779123
}
```

The runner captures one UTC wall-clock anchor and one `Instant` anchor at run
start. Point timestamps are derived as wall-clock anchor plus monotonic elapsed
time. This keeps successful point timestamps nondecreasing even if the system
clock changes during the run. Wall-clock values are metadata only and must
never participate in latency, bandwidth, jitter, percentile, early-stopping,
timeout, or cancellation calculations.

Loaded-latency points use the same timestamp field. When the latest-20 retention
rule evicts a point, its timestamp is evicted with it.

## Text output

When metadata succeeds, add this compact block after protocol and before speed
metrics:

```text
Edge: IST — Arnavutkoy, TR
Network: TurkNet Iletisim Hizmetleri A.S. (AS12735)
Public IP: 2a02:ff0:...
Measured at: 2026-07-19T09:02:59.123Z
```

Formatting rules:

- prefer `colo — city, country_code` for the edge;
- omit missing edge components without placeholder punctuation;
- prefer `organization (ASnumber)` for the network;
- show whichever network component is available if only one exists;
- print the public IP verbatim;
- always print `Measured at` because it is local run metadata;
- when metadata is enabled but unavailable, print `Metadata: unavailable` and
  still print `Measured at`;
- when `--no-metadata` is set, omit Edge, Network, Public IP, and Metadata lines.

Raw point arrays remain JSON-only. No charts, box plots, cursor control, or TUI
are added.

## Architecture

Preserve existing boundaries:

- `cli` adds and validates `--no-metadata`.
- `config` carries the validated metadata policy.
- `transport` owns the bounded `/meta` HTTP request and response decoding.
- `runner` owns request ordering, nonfatal enrichment, run clock anchors, and
  point timestamp assignment.
- `results` owns serializable metadata and timestamp fields.
- `output` renders the compact text block and unchanged one-document JSON.

The metadata response type must not leak reqwest types into `runner`, `results`,
or `output`. The measurement/statistics APIs remain independent of reqwest.

## Error and cancellation behavior

- `--no-metadata` performs zero metadata I/O.
- Metadata HTTP status, timeout, body, size-limit, and JSON errors become
  diagnostics, not `RunnerError` terminal failures.
- Ctrl+C during measurements cancels the run and skips metadata enrichment.
- Ctrl+C during the post-plan metadata request cancels that request and returns
  the already-completed measurement outcome as cancelled, preserving points.
- No metadata request is retried.
- A malformed optional field does not discard otherwise valid metadata; that
  leaf becomes `null` and a diagnostic may be recorded.
- Invalid/non-finite coordinates become `null` and never reach JSON.

## Testing

Use TDD and local fixtures. Ordinary tests must not call Cloudflare.

Required deterministic coverage:

1. Clap exposes `--no-metadata`; the flag maps to `RunConfig` and defaults off.
2. A local `/meta` response maps camelCase Cloudflare fields into the stable
   snake_case result model, including IPv6, ASN, client location, and edge.
3. Unknown response fields are ignored and missing leaves serialize as `null`.
4. Non-finite or wrongly typed coordinates are rejected per leaf without
   contaminating JSON.
5. The response body rejects data beyond 64 KiB without unbounded buffering.
6. Metadata is requested once, after all measurement-plan requests.
7. `--no-metadata` causes zero `/meta` requests and `metadata: null`.
8. Metadata failure preserves successful speed points, yields a diagnostic,
   and does not change the process exit status.
9. Cancellation joins measurement/probe tasks and does not leave a metadata
   request detached.
10. Every accepted raw point has a nondecreasing `measured_at_unix_ms` value;
    reductions remain unchanged when timestamps differ.
11. Text output covers complete, partial, unavailable, and disabled metadata
    without malformed punctuation.
12. JSON mode still writes exactly one parseable document to stdout.
13. The existing upstream measurement-plan fixture and request-count tests
    distinguish the post-plan metadata request from measurement traffic.

Add one ignored live metadata test that validates only broad response shape and
does not expose the returned public IP in assertion messages or committed
fixtures.

## Documentation changes

Update in the same implementation change:

- `README.md` usage, privacy disclosure, text example, JSON example, and data
  semantics;
- `docs/PRD.md` CLI and result requirements;
- `docs/MVP.md` acceptance criteria;
- `docs/MEASUREMENT_COMPATIBILITY.md` post-plan metadata ordering and wall-clock
  non-participation;
- `docs/TEST_STRATEGY.md` local and ignored-live metadata coverage.

## Non-goals

- packet loss or TURN/WebRTC;
- AIM/network-quality scores;
- maps, charts, a TUI, or browser rendering;
- third-party IP or ASN services;
- historical storage or result upload;
- changing the Cloudflare measurement plan;
- using wall-clock time in any speed or latency calculation.

## Acceptance criteria

The feature is complete when default text and JSON results contain the requested
metadata and timestamps, `--no-metadata` performs no metadata request, metadata
failures remain nonfatal, raw point reductions are unchanged, all existing and
new tests pass, and the repository's formatting, Clippy, test, and release-build
gates succeed.
