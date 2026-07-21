# cfbench

`cfbench` is an unofficial native Rust command-line speed test that measures a connection against Cloudflare's speed-test endpoints. It follows Cloudflare Speedtest's public measurement schedule and reductions without embedding a browser or providing a TUI.

`cfbench` is not affiliated with, endorsed by, or supported by Cloudflare.

This repository is pre-release. MVP implementation is present, but release
validation evidence is still being collected.

## Install from source

Rust 1.95 or newer is required.

```bash
git clone https://github.com/fcendesu/cfbench.git
cd cfbench
cargo install --path .
```

To build without installing:

```bash
cargo build --release
./target/release/cfbench --help
```

On Windows, the executable is `target\release\cfbench.exe`.

## Usage

Run the default Cloudflare-compatible plan:

```bash
cfbench
```

```bash
cfbench --ipv4
cfbench --ipv6 --no-upload
cfbench --no-loaded-latency
cfbench --no-metadata
cfbench --json > result.json
cfbench --quiet --timeout 60
```

Options:

```text
      --ipv4                 Use IPv4 only
      --ipv6                 Use IPv6 only
      --json                 Emit versioned JSON to stdout
      --no-download          Skip download measurements
      --no-upload            Skip upload measurements
      --no-loaded-latency    Disable latency probes during transfers
      --no-metadata          Skip the default public IP and network metadata request
      --timeout <SECONDS>    Per-request timeout [default: 30]
  -q, --quiet                Suppress progress lines
  -h, --help                 Print help
  -V, --version              Print version
```

`--ipv4` and `--ipv6` are mutually exclusive. Forced-family modes bypass system
proxies because a proxy cannot guarantee the target connection's address
family. Auto mode retains standard system proxy behavior. The timeout accepts
1 through 300 seconds and is an absolute deadline for the complete request,
including response-body streaming. Progress and diagnostics go to stderr;
`--json` writes exactly one JSON document to stdout and suppresses all progress.
`--quiet` also suppresses every progress line, but not the final result,
diagnostics, or fatal errors.

Metadata collection is enabled by default. One post-test request to Cloudflare's
`/meta` endpoint collects the public IP, ASN, network organization, and
approximate location already visible to Cloudflare. `--no-metadata` skips the
request entirely and omits those lines from text output; JSON then reports
`metadata_status: "disabled"` and `metadata: null`.

## Progress output

Ordinary text mode writes stable UTF-8 progress lines to stderr as individual
requests complete. Loaded-latency probes may interleave with transfer results:

```text
Testing against Cloudflare edge...
[latency 1/20] 22.80 ms
[download 100 KB 1/9] 91.42 Mbps — 11.0 ms
[loaded/download 1] 25.40 ms
[upload 1 MB 1/6] 328.09 Mbps — 24.5 ms
[loaded/upload 1] 26.60 ms
[download 100 MB 1/3] failed — HTTP 403
[download] larger payload groups skipped — request duration threshold reached
[packet loss] unavailable — TURN not configured
```

The renderer uses no ANSI animation, cursor movement, carriage-return rewriting,
or per-chunk output. Progress delivery uses a bounded nonblocking channel; if
stderr stalls, a progress event may be dropped rather than delaying a measured
request. Stored points, reductions, usage accounting, final output, and the
terminal `error:` diagnostic are unaffected.

## Text output

The ordinary line-oriented renderer produces output in this form:

```text
cfbench 0.1.0
Target: Cloudflare edge
Protocol: IPv6 / HTTP/2
Edge: IST — Arnavutkoy, TR
Network: Example Network (AS64496)
Public IP: 2001:db8::1
Measured at: 2026-07-19T09:02:59.123Z

Idle latency: 14.82 ms
Idle jitter: 1.74 ms
Download: 842.16 Mbps
Download latency: 32.41 ms
Download jitter: 4.88 ms
Upload: 47.62 Mbps
Upload latency: 55.09 ms
Upload jitter: 8.31 ms
Packet loss: unavailable

Downloaded: 418.7 MB
Uploaded: 83.4 MB
Duration: 16.42 s
```

Mbps and MB are decimal units. Unmeasured and unsupported values display as
`unavailable`. If metadata collection fails, text prints
`Metadata: unavailable` and the locally captured `Measured at` timestamp; the speed-test
result remains usable. With `--no-metadata`, Edge, Network, Public IP, and
Metadata lines are omitted while `Measured at` remains. Packet loss remains
unavailable in the MVP because Cloudflare's packet-loss method requires
TURN/WebRTC; `cfbench` does not substitute ICMP loss. AIM/network-quality scores
are also outside the MVP.

## JSON output

`cfbench --json` emits one schema-v1 document including target metadata, the run
timestamp, and the raw points used by summary reductions. This abbreviated
successful shape uses documentation-only network values:

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
    "duration_ms": 16420.0
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
  },
  "packet_loss": {
    "status": "unavailable",
    "reason": "turn_not_implemented",
    "ratio": null
  },
  "failures": [],
  "diagnostics": []
}
```

`metadata_status` is `available` for an accepted bounded `/meta` object,
`unavailable` when enabled collection fails, and `disabled` when
`--no-metadata` prevents the request. `target.metadata` is `null` for the last
two states; all metadata leaves are independently nullable when available.

Every accepted raw latency or bandwidth point has `measured_at_unix_ms`.
Bandwidth arrays remain grouped by direction, and `requested_bytes` is the
canonical payload-group key; points are not duplicated into a second grouped
shape. Loaded-latency points remain separate by direction and retain only the
latest 20. The initial one-packet estimate is orchestration-only and is not
serialized. Missing measurements serialize as `null`, never as zero. A failed
or cancelled run preserves completed points and exits non-zero; diagnostics and
the terminal error go to stderr.

## Data use

The default plan ramps through large payloads. If every scheduled transfer runs, it requests about 969 MB of download payload and sends about 297 MB of upload payload. Adaptive stopping can reduce that total, but users on metered or constrained connections should use `--no-download` or `--no-upload` as appropriate. Loaded-latency probes request zero-byte response bodies.

No project telemetry or result-collection service is used. Measurement traffic
goes directly to Cloudflare's `__down` and `__up` endpoints. Unless disabled,
one `/meta` request follows the completed timed plan. It does not warm the
connection before testing or overlap a measurement, and its body bytes and
elapsed time are excluded from `usage`. A metadata failure is nonfatal and is
reported once as a diagnostic without fabricating a point.

Ctrl+C during the active post-plan metadata request makes cancellation the
terminal outcome. Completed points and any earlier measurement failure remain
in `failures`, the metadata cancellation is appended there, and it is not
substituted with a metadata diagnostic.

Downloaded usage counts response-body bytes actually received, including bytes
received before a failed or cancelled request. Uploaded usage counts bytes
yielded to reqwest's HTTP request body, the closest native observable boundary;
it does not claim that the remote peer accepted every yielded byte.

`started_at` is captured immediately before the first plan step. Point
completion timestamps are derived from that UTC anchor plus monotonic elapsed
time, so they remain nondecreasing even if the system wall clock changes.
Neither timestamp participates in latency, bandwidth, jitter, percentiles,
adaptive stopping, timeouts, cancellation, or usage duration.

## Compatibility and timing

The implementation is pinned to Cloudflare Speedtest `v1.11.0`, upstream commit [`cfc99a74fd8d5c2121d319aeb7894c6246202c65`](https://github.com/cloudflare/speedtest/commit/cfc99a74fd8d5c2121d319aeb7894c6246202c65).

The schedule, thresholds, percentile rules, server-time handling, and loaded-latency behavior follow that baseline. Results are not expected to be numerically identical to `speed.cloudflare.com`: the browser implementation observes `PerformanceResourceTiming`, while `cfbench` measures native reqwest request, response-header, and streamed-body boundaries with monotonic time.

Cloudflare's large download endpoint rejected a context-free 100 MB request
with HTTP 403 during live validation on 2026-07-19. Production latency and
download GETs therefore send a normalized same-origin `Referer`; upload POSTs
send that `Referer` plus a scheme-and-authority-only `Origin`. These values
contain no credentials, query, or fragment and are constructed before request
timing starts. The transport never retries a rejected or failed measurement,
including HTTP 403 and 429 responses.

See [Measurement Compatibility](docs/MEASUREMENT_COMPATIBILITY.md) for exact rules and native equivalents.

## Architecture

- `plan` defines the immutable versioned measurement schedule.
- `transport` performs streaming reqwest requests without automatic retries.
- `runner` executes the plan, adaptive stopping, loaded probes, and cancellation.
- `measurement` and `statistics` convert and reduce raw observations.
- `results` owns the stable schema; `output` renders text and JSON.

Downloads are consumed incrementally rather than concatenated in memory. Uploads use a reusable bounded chunk stream rather than allocating a fresh payload-sized body for every request.

## Development

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
cargo build --release
```

Live endpoint tests are ignored by default because they require Internet access
and consume network resources. Run them explicitly with
`cargo test --test live_cloudflare -- --ignored`. The large-download
request-context guard reads only the 100 MB response headers and drops the body;
it can be run alone with
`cargo test --lib transport::reqwest_transport::tests::live_large_download_accepts_browser_request_context -- --ignored --exact`.
The metadata-only broad-shape guard can be run without executing transfer tests:
`cargo test --test live_cloudflare live_cloudflare_metadata_has_broad_public_shape -- --ignored --exact`.

Requirements, design, testing, and the reqwest decision are documented under [`docs/`](docs/).

## License

Licensed under the [MIT License](LICENSE).
