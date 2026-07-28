# cfbench

`cfbench` is a lightweight, native Rust command-line speed test for Cloudflare's edge network. It measures download and upload bandwidth, idle latency and jitter, and latency while the connection is loaded. It has no TUI, browser runtime, telemetry service, or local database.

`cfbench` is unofficial and is not affiliated with, endorsed by, or supported by Cloudflare.

## Install (Linux x86_64)

### Bash installer for `.deb` and `.rpm`

```bash
curl -fsSL https://raw.githubusercontent.com/fcendesu/cfbench/main/scripts/install.sh | sh
```

### Build from source

Rust 1.95 or newer is required:

```bash
git clone https://github.com/fcendesu/cfbench.git
cd cfbench
cargo install --path .
```

## Usage

```bash
cfbench
cfbench --ipv4
cfbench --ipv6 --no-upload
cfbench --no-loaded-latency
cfbench --verbose --no-metadata
cfbench --json > result.json
cfbench --quiet --timeout 60
```

Run `cfbench --help` for the complete option list. Progress and diagnostics use stderr. `--verbose` enables live progress. `--json` writes exactly one schema-v1 JSON document to stdout and suppresses progress; `--quiet` also suppresses progress.

For automation, cfbench exits `0` after a complete measurement, `2` after a
partial measurement with at least one accepted latency, download, or upload
point, and `1` when no usable measurement was accepted or the run is cancelled.

### Example output

The values below are illustrative and will vary by connection. `--no-metadata` keeps public IP, network, and edge-location information out of the result.

```text
$ cfbench --verbose --no-metadata
Testing against Cloudflare edge...
[latency 1/1] 18.42 ms
[download 100 KB 1/1] 71.25 Mbps — 11.2 ms
...

cfbench 0.1.1
Target: Cloudflare edge
Protocol: IPv6 / HTTP/1.1
Metadata: disabled
Measured at: 2026-07-26T12:00:00.000Z

Idle latency: 17.86 ms
Idle jitter: 1.44 ms
Download: 742.18 Mbps
Download latency: 29.73 ms
Download jitter: 4.10 ms
Upload: 216.94 Mbps
Upload latency: 47.81 ms
Upload jitter: 7.62 ms

Downloaded: 469.0 MB
Uploaded: 296.8 MB
Duration: 24.18 s
```

## What it measures

- Idle latency and jitter
- Download and upload bandwidth
- Download-loaded and upload-loaded latency and jitter
- Transferred payload bytes, duration, negotiated HTTP version, and raw measurement points
- Optional Cloudflare `/meta` context: public IP, ASN, network organization, and edge/location data

`--no-metadata` skips the `/meta` request completely. The tool does not collect or send results to a cfbench service.

Packet loss is deliberately not displayed or measured in the 0.1 release series. Cloudflare's metric requires TURN/WebRTC; cfbench does not substitute ICMP loss or present an incompatible value as packet loss.

## Compatibility

cfbench follows Cloudflare Speedtest `v1.12.1` for its request order, payload sizes, thresholds, percentile reductions, and loaded-latency rules. Native Rust timing cannot exactly reproduce the browser's `PerformanceResourceTiming` boundaries or browser-only TCP connection calibration, so results are methodology-compatible rather than numerically identical.

See [installation details](docs/INSTALLATION.md) and [measurement compatibility](docs/COMPATIBILITY.md).

## License

Licensed under the [MIT License](LICENSE).
