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

### FR-7: JSON output

`--json` must emit exactly one JSON document to stdout. Progress and diagnostics must go to stderr or be suppressed.

### FR-8: Partial-feature reporting

Features that are intentionally unsupported, such as packet loss in the MVP, must be represented as `unavailable` or `null`, not fabricated from a different protocol.

### FR-9: Timeouts and cancellation

Every network operation must have explicit bounds. Ctrl+C must cancel active work and exit without leaving background tasks running.

### FR-10: Data-use visibility

Final output must show total payload bytes downloaded and uploaded. JSON must expose these as integers.

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
      --timeout <SECONDS>    Per-request timeout
  -q, --quiet                Suppress progress lines
  -h, --help                 Print help
  -V, --version              Print version
```

The default invocation is `cfbench` with automatic IP-family selection and Cloudflare-compatible loaded-latency probes enabled.

## 9. Output requirements

Example text output:

```text
cfbench 0.1.0
Target: Cloudflare edge
Protocol: IPv6 / HTTP/2

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

## 10. JSON contract

The top-level schema must be versioned from the first release:

```json
{
  "schema_version": 1,
  "client": { "name": "cfbench", "version": "0.1.0" },
  "target": {
    "provider": "cloudflare",
    "ip_family": "ipv6",
    "http_version": "2"
  },
  "summary": {
    "unloaded_latency_ms": 14.82,
    "unloaded_jitter_ms": 1.74,
    "download_bps": 842160000,
    "download_loaded_latency_ms": 32.41,
    "download_loaded_jitter_ms": 4.88,
    "upload_bps": 47620000,
    "upload_loaded_latency_ms": 55.09,
    "upload_loaded_jitter_ms": 8.31,
    "packet_loss_ratio": null
  },
  "usage": {
    "download_payload_bytes": 418700000,
    "upload_payload_bytes": 83400000,
    "duration_ms": 16420
  },
  "points": {
    "latency": [],
    "download": [],
    "upload": [],
    "download_loaded_latency": [],
    "upload_loaded_latency": []
  }
}
```

Additive fields may be introduced without changing `schema_version`; breaking changes require a new schema version.

Target metadata aggregates every successful HTTP observation. A single observed value is serialized normally, differing non-null values are serialized as `"mixed"`, missing observations are ignored, and a run with no observed value serializes `null`.

## 11. Error behavior

- Invalid arguments: handled by Clap with exit code `2`.
- Successful test: exit code `0`.
- Network or measurement failure that prevents a valid summary: exit code `1`.
- Packet loss being unavailable is not an error in the MVP.
- If one enabled bandwidth direction fails, the tool should print or serialize completed partial measurements and return exit code `1`.
- Error messages must identify the stage, endpoint, and underlying error without printing secrets.
- Endpoint context includes only scheme, authority, and path; measurement query parameters and URL credentials must not be printed.
- Result diagnostics are written to stderr in text and JSON modes, including with `--quiet`; quiet mode suppresses progress only.

## 12. Quality requirements

- No panics on malformed headers, timeouts, empty result sets, or interrupted streams.
- Statistics must be deterministic and separately unit tested.
- The test runner must not hold complete 100 MB or 250 MB download bodies in memory.
- Upload payload generation must avoid repeated large allocations.
- Output formatting must not affect timing-critical tasks.
- The binary must avoid telemetry and must not send results anywhere except requests required by the configured speed-test target.

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
