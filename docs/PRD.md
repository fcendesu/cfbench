# Product Requirements Document: cfbench

- Status: Implemented; release validation pending
- Date: 2026-07-19
- Product: `cfbench`
- Initial release: `0.1.0`

## 1. Summary

`cfbench` is an unofficial, native Rust command-line speed test that measures latency, jitter, download bandwidth, upload bandwidth, and loaded latency against Cloudflare's edge network.

The tool is intended for servers, terminals, scripts, CI jobs, home labs, and network troubleshooting environments where a browser is unavailable or undesirable. It should follow Cloudflare Speedtest's public measurement sequence and calculation rules as closely as possible without embedding or launching a browser.

## 2. Problem

Cloudflare provides a browser-based speed test and a JavaScript measurement engine. A user who is connected to a headless Linux server, SSH session, container host, or minimal operating system cannot run the same workflow as a small native command.

Existing generic speed-test CLIs may target a different provider, use different server-selection behavior, or apply different measurement rules. Users therefore cannot easily compare terminal measurements with the behavior of Cloudflare's browser test.

## 3. Product goals

1. Run a Cloudflare-focused connection-quality test from a terminal with one command.
2. Reproduce Cloudflare's published default latency, download, upload, and loaded-latency methodology.
3. Produce useful human-readable output without a TUI.
4. Produce stable machine-readable JSON for automation and historical collection.
5. Support Linux, macOS, and Windows from a single Rust codebase.
6. Clearly disclose differences between browser timing and native HTTP timing.

## 4. Non-goals

The initial product will not:

- claim to be an official Cloudflare product;
- guarantee numerically identical results to `speed.cloudflare.com`;
- select a Cloudflare data center manually, because Cloudflare routing is anycast-based;
- embed Chromium or execute the official browser JavaScript engine;
- provide a full-screen terminal UI;
- provide a web dashboard or hosted result service;
- upload results to a `cfbench` backend;
- calculate Cloudflare AIM quality scores in the MVP;
- substitute ICMP loss for Cloudflare's TURN-based packet-loss method.

## 5. Target users

### Headless server operator

Runs a quick test over SSH to diagnose routing, ISP performance, upload saturation, or IPv4/IPv6 differences.

### Home-lab user

Tests a self-hosted server and records structured results before and after network changes.

### Developer or SRE

Uses JSON output in scripts, diagnostics, CI environments, or periodic monitoring.

### Network enthusiast

Compares unloaded and loaded latency to identify bufferbloat and connection responsiveness.

## 6. Primary user stories

- As a user, I can run `cfbench` and receive latency, jitter, download, upload, and loaded-latency results.
- As a user, I can force IPv4 or IPv6 so that I can compare paths.
- As a user, I can request JSON output without progress text contaminating stdout.
- As a user, I can disable download or upload when I need to limit data consumption.
- As a user, I can see how much data the test transferred and how long it ran.
- As a user, I can see the responding Cloudflare edge and the public network
  context Cloudflare observes, or disable that metadata request for privacy.
- As a user, I receive a clear error when the endpoint cannot be reached or the requested IP family is unavailable.
- As a developer, I can inspect individual measurement points in JSON when debugging calculation differences.

## 7. Functional requirements

### FR-1: Default test

`cfbench` must run the published Cloudflare default sequence for idle latency, download, upload, and loaded latency, excluding TURN packet loss in the MVP.

### FR-2: Latency and jitter

The tool must report:

- unloaded latency;
- unloaded jitter;
- download-loaded latency and jitter when enough points exist;
- upload-loaded latency and jitter when enough points exist.

### FR-3: Bandwidth

The tool must report download and upload bandwidth in Mbps while retaining raw bps values in structured output.

### FR-4: Adaptive ramp-up

The runner must stop later measurement groups in a direction after the upstream finish-duration condition has been met, while honoring the initial bypass measurement.

### FR-5: IP-family control

The CLI must support automatic selection, IPv4-only, and IPv6-only modes. `--ipv4` and `--ipv6` must be mutually exclusive.

### FR-6: Plain terminal output

The normal output must consist of ordinary lines. It may show stage progress but must not use an alternate screen, cursor-addressed interface, interactive widgets, or a TUI framework.

Ordinary text mode writes the opening status and individual request progress to
stderr. Successful unloaded-latency, transfer, and loaded-latency requests use
phase-local, payload-group-local, and direction-local counters respectively.
Failures, adaptive-stop notices, and the unsupported packet-loss stage are
reported immediately with safe categories. Progress uses a bounded nonblocking
channel so a stalled stderr consumer cannot delay measurement timing or alter
stored points.

### FR-7: JSON output

`--json` must emit exactly one JSON document to stdout and suppress every
progress line. Diagnostics remain on stderr.

### FR-8: Partial-feature reporting

Features that are intentionally unsupported, such as packet loss in the MVP, must be represented as `unavailable` or `null`, not fabricated from a different protocol.

### FR-9: Timeouts and cancellation

Every network operation must have explicit bounds. Ctrl+C must cancel active work and exit without leaving background tasks running.

### FR-10: Data-use visibility

Final output must show total payload bytes downloaded and uploaded. JSON must expose these as integers.

### FR-11: Cloudflare request context

Latency and download GETs must send a normalized same-origin `Referer`. Upload
POSTs must send that `Referer` plus an `Origin` containing only scheme and
authority. Neither header may contain credentials, a query, or a fragment.
Request construction must remain outside the measured interval, and no failed
measurement request may be retried.

### FR-12: Result metadata and timestamps

Metadata collection is enabled by default. After every timed measurement-plan
step and loaded probe has stopped, the runner must request `/meta` exactly once
through the same configured client. The request must not precede or overlap the
published plan, warm the connection before measurement, or count toward
payload usage or plan duration. `--no-metadata` must skip the request rather
than merely hiding its result.

Results must include the run's UTC start time plus a completion timestamp for
every accepted raw point. These values are descriptive metadata derived from a
wall-clock/monotonic anchor and must not affect any timing, reduction, schedule,
timeout, cancellation, or usage calculation. Metadata collection failures are
nonfatal and must not change the measurement outcome or fabricate points.

## 8. CLI requirements

Required MVP interface:

```text
cfbench [OPTIONS]

Options:
      --ipv4                 Use IPv4 only
      --ipv6                 Use IPv6 only
      --json                 Emit versioned JSON to stdout
      --no-download          Skip download measurements
      --no-upload            Skip upload measurements
      --no-loaded-latency    Disable latency probes during transfers
      --no-metadata          Do not request or display public IP and network metadata
      --timeout <SECONDS>    Per-request timeout
  -q, --quiet                Suppress progress lines
  -h, --help                 Print help
  -V, --version              Print version
```

The default invocation is `cfbench` with automatic IP-family selection,
Cloudflare-compatible loaded-latency probes, and post-plan metadata collection
enabled.

## 9. Output requirements

Example text output:

```text
cfbench 0.1.0
Target: Cloudflare edge
Protocol: IPv6 / HTTP/2
Edge: IST — Arnavutkoy, TR
Network: Example Network (AS64496)
Public IP: 2001:db8::1
Measured at: 2026-07-19T09:02:59.123Z

Idle latency:       14.82 ms
Idle jitter:         1.74 ms
Download:          842.16 Mbps
Download latency:   32.41 ms
Download jitter:     4.88 ms
Upload:              47.62 Mbps
Upload latency:      55.09 ms
Upload jitter:        8.31 ms
Packet loss:       unavailable

Downloaded:         418.7 MB
Uploaded:            83.4 MB
Duration:            16.42 s
```

The display must not imply a manually selected data center. Edge metadata may be shown only when obtained reliably and must be marked as informational.

When metadata is enabled but cannot be collected, the text renderer writes
`Metadata: unavailable` and still writes `Measured at`. When collection is
disabled, it omits Edge, Network, Public IP, and Metadata lines. Metadata
strings must be joined without dangling separators when optional components are
missing.

Progress is separate from the final summary and uses these exact line forms:

```text
Testing against Cloudflare edge...
[latency 1/20] 22.80 ms
[download 100 KB 1/9] 91.42 Mbps — 11.0 ms
[loaded/download 1] 25.40 ms
[upload 1 MB 1/6] 328.09 Mbps — 24.5 ms
[download 100 MB 1/3] failed — HTTP 403
[download] larger payload groups skipped — request duration threshold reached
[packet loss] unavailable — TURN not configured
```

`--quiet` and `--json` suppress the opening line and every request/stage
progress line. They do not suppress final results, diagnostics, or fatal errors.

## 10. JSON contract

The top-level schema must be versioned from the first release:

```json
{
  "schema_version": 1,
  "started_at": "2026-07-19T09:02:59.123Z",
  "client": { "name": "cfbench", "version": "0.1.0" },
  "target": {
    "provider": "cloudflare",
    "ip_family": "ipv6",
    "http_version": "2",
    "timing_model": "native_reqwest_v1",
    "metadata_status": "available",
    "metadata": {
      "public_ip": "2001:db8::1",
      "asn": 64496,
      "as_organization": "Example Network",
      "client_location": {
        "country_code": "TR",
        "city": "Istanbul",
        "region": "Istanbul",
        "postal_code": null,
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
  },
  "summary": {
    "unloaded_latency_ms": 14.82,
    "unloaded_jitter_ms": null,
    "download_bps": 842160000,
    "download_loaded_latency_ms": null,
    "download_loaded_jitter_ms": null,
    "upload_bps": null,
    "upload_loaded_latency_ms": null,
    "upload_loaded_jitter_ms": null,
    "packet_loss_ratio": null
  },
  "usage": {
    "download_payload_bytes": 10000000,
    "upload_payload_bytes": 0,
    "duration_ms": 16420
  },
  "points": {
    "latency": [
      {
        "ping_ms": 14.82,
        "ttfb_ms": 24.82,
        "server_time_ms": 10.0,
        "http_version": "2",
        "measured_at_unix_ms": 1784451779123
      }
    ],
    "download": [
      {
        "direction": "download",
        "requested_bytes": 10000000,
        "payload_bytes": 10000000,
        "duration_ms": 105.47,
        "adjusted_duration_ms": 95.47,
        "ping_ms": 14.82,
        "server_time_ms": 10.0,
        "bps": 842160000,
        "http_version": "2",
        "measured_at_unix_ms": 1784451780345
      }
    ],
    "upload": [],
    "download_loaded_latency": [],
    "upload_loaded_latency": []
  }
}
```

Additive fields may be introduced without changing `schema_version`; breaking changes require a new schema version.

Protocol target fields aggregate every successful measurement HTTP observation.
A single observed value is serialized normally, differing non-null values are
serialized as `"mixed"`, missing observations are ignored, and a run with no
observed value serializes `null`.

`target.metadata_status` has three values: `available` means one bounded valid
Cloudflare `/meta` object was accepted, `unavailable` means enabled collection
failed, and `disabled` means `--no-metadata` skipped all metadata I/O.
`target.metadata` is `null` for unavailable and disabled; when available, every
leaf remains nullable independently.

Each accepted latency or bandwidth point includes `measured_at_unix_ms`.
Bandwidth points stay in direction arrays and use `requested_bytes` as the
canonical payload-group key; the schema does not duplicate them in a grouped
view. Download-loaded and upload-loaded latency arrays remain separate, and the
initial one-packet latency estimate is not public raw output.

## 11. Error behavior

- Invalid arguments: handled by Clap with exit code `2`.
- Successful test: exit code `0`.
- Network or measurement failure that prevents a valid summary: exit code `1`.
- Packet loss being unavailable is not an error in the MVP.
- If one enabled bandwidth direction fails, the tool should print or serialize completed partial measurements and return exit code `1`.
- Error messages must identify the stage, endpoint, and underlying error without printing secrets.
- Endpoint context includes only scheme, authority, and path; measurement query parameters and URL credentials must not be printed.
- Result diagnostics are written to stderr in text and JSON modes, including with `--quiet`; quiet mode suppresses progress only.
- HTTP 403, 429, timeout, and stream failures are terminal for their scheduled
  measurement request and are never retried.
- Metadata HTTP, timeout, body-limit, JSON, and top-level-structure failures are
  nonfatal. They produce `metadata_status: "unavailable"`, `metadata: null`,
  and one redacted stderr diagnostic while preserving the measurement-derived
  exit status. Malformed optional leaves become null without discarding an
  otherwise valid object. Cancellation during metadata remains a cancelled run.

Live validation on 2026-07-19 found that a context-free 100 MB Cloudflare
download returned HTTP 403 after earlier groups had successfully transferred
169 MB. Adding either same-origin `Referer` or `Origin` made the headers request
succeed; production GET behavior uses `Referer` to match browser request
semantics without browser User-Agent impersonation. An ignored live acceptance
guard checks the 100 MB response headers and drops the body immediately.

A separate ignored live metadata guard validates only a nonempty public-IP, an
ASN greater than zero within the `u32` result type, and a
three-uppercase-letter colo predicate. It must not record returned
public/network/location values in fixtures, assertion messages, or successful
output.

## 12. Quality requirements

- No panics on malformed headers, timeouts, empty result sets, or interrupted streams.
- Statistics must be deterministic and separately unit tested.
- The test runner must not hold complete 100 MB or 250 MB download bodies in memory.
- Upload payload generation must avoid repeated large allocations.
- Output formatting must not affect timing-critical tasks.
- The binary must avoid telemetry and must not send results anywhere except requests required by the configured speed-test target.
- Default metadata collection discloses the public IP, ASN, and approximate
  location already visible to Cloudflare; help and user documentation must
  explain the `--no-metadata` opt-out.

## 13. Success metrics

MVP success is reached when:

1. The default measurement plan and thresholds match the documented upstream baseline.
2. The CLI completes on Linux, macOS, and Windows.
3. IPv4-only and IPv6-only modes work on compatible hosts.
4. JSON is valid and stable across repeated runs.
5. Local deterministic tests cover all calculation and early-stop rules.
6. Repeated side-by-side runs are directionally consistent with Cloudflare's browser test, with documented expected differences.
7. No known transfer-size-proportional memory growth occurs during downloads.

## 14. Legal and naming

`cfbench` must state that it is unofficial and not affiliated with or endorsed by Cloudflare. Cloudflare names and endpoint references are descriptive compatibility references. The project should not use Cloudflare's logo or visual identity.

## 15. References

- Cloudflare Speedtest repository: <https://github.com/cloudflare/speedtest>
- Cloudflare Speedtest README and public defaults: <https://github.com/cloudflare/speedtest/blob/main/README.md>
- Browser resource timing model: <https://developer.mozilla.org/en-US/docs/Web/API/PerformanceResourceTiming>
- reqwest documentation: <https://docs.rs/reqwest/latest/reqwest/>
