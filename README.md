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

Mbps and MB are decimal units. Unmeasured and unsupported values display as `unavailable`. Packet loss remains unavailable in the MVP because Cloudflare's packet-loss method requires TURN/WebRTC; `cfbench` does not substitute ICMP loss.

## JSON output

`cfbench --json` emits the versioned result model, including raw points used by the summary reductions. This is the renderer's unavailable-result shape; a successful run fills the summary, usage, target metadata, and point arrays.

```json
{
  "schema_version": 1,
  "client": { "name": "cfbench", "version": "0.1.0" },
  "target": {
    "provider": "cloudflare",
    "ip_family": null,
    "http_version": null,
    "timing_model": "native_reqwest_v1"
  },
  "summary": {
    "unloaded_latency_ms": null,
    "unloaded_jitter_ms": null,
    "download_bps": null,
    "download_loaded_latency_ms": null,
    "download_loaded_jitter_ms": null,
    "upload_bps": null,
    "upload_loaded_latency_ms": null,
    "upload_loaded_jitter_ms": null,
    "packet_loss_ratio": null
  },
  "usage": {
    "download_payload_bytes": 0,
    "upload_payload_bytes": 0,
    "duration_ms": 0.0
  },
  "points": {
    "latency": [],
    "download": [],
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

Missing measurements serialize as `null`, never as zero. A failed or cancelled run preserves completed points and exits non-zero; diagnostics and the terminal error go to stderr.

## Data use

The default plan ramps through large payloads. If every scheduled transfer runs, it requests about 969 MB of download payload and sends about 297 MB of upload payload. Adaptive stopping can reduce that total, but users on metered or constrained connections should use `--no-download` or `--no-upload` as appropriate. Loaded-latency probes request zero-byte response bodies.

No project telemetry or result-collection service is used. Test traffic goes directly to Cloudflare's `__down` and `__up` endpoints.

Downloaded usage counts response-body bytes actually received, including bytes
received before a failed or cancelled request. Uploaded usage counts bytes
yielded to reqwest's HTTP request body, the closest native observable boundary;
it does not claim that the remote peer accepted every yielded byte.

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

Requirements, design, testing, and the reqwest decision are documented under [`docs/`](docs/).

## License

Licensed under the [MIT License](LICENSE).
