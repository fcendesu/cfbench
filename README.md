# cfbench

`cfbench` is a lightweight, native Rust command-line speed test for Cloudflare's edge network. It measures download and upload bandwidth, idle latency and jitter, and latency while the connection is loaded. It has no TUI, browser runtime, telemetry service, or local database.

`cfbench` is unofficial and is not affiliated with, endorsed by, or supported by Cloudflare.

## Install

Linux x86_64 releases provide a Debian package, RPM package, and a standalone binary. The installer downloads the standalone binary and verifies its published SHA-256 checksum before installing it to `~/.local/bin`:

```bash
curl -fsSL https://raw.githubusercontent.com/fcendesu/cfbench/main/scripts/install.sh | sh
```

Install a specific version or choose a directory:

```bash
CFBENCH_VERSION=0.1.0 curl -fsSL https://raw.githubusercontent.com/fcendesu/cfbench/main/scripts/install.sh | sh
curl -fsSL https://raw.githubusercontent.com/fcendesu/cfbench/main/scripts/install.sh | sudo env CFBENCH_INSTALL_DIR=/usr/local/bin sh
```

For a package-manager install, download the `.deb` or `.rpm` from the matching [GitHub Release](https://github.com/fcendesu/cfbench/releases), verify it against the release checksum file, then install it with your distribution's package manager.

To install from source, Rust 1.95 or newer is required:

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
cfbench --no-metadata
cfbench --json > result.json
cfbench --quiet --timeout 60
```

Run `cfbench --help` for the complete option list. Progress and diagnostics use stderr. `--json` writes exactly one schema-v1 JSON document to stdout and suppresses progress; `--quiet` also suppresses progress.

## What it measures

- Idle latency and jitter
- Download and upload bandwidth
- Download-loaded and upload-loaded latency and jitter
- Transferred payload bytes, duration, negotiated HTTP version, and raw measurement points
- Optional Cloudflare `/meta` context: public IP, ASN, network organization, and edge/location data

`--no-metadata` skips the `/meta` request completely. The tool does not collect or send results to a cfbench service.

Packet loss is deliberately not displayed or measured in 0.1.0. Cloudflare's metric requires TURN/WebRTC; cfbench does not substitute ICMP loss or present an incompatible value as packet loss.

## Compatibility

cfbench follows Cloudflare Speedtest `v1.12.1` for its request order, payload sizes, thresholds, percentile reductions, and loaded-latency rules. Native Rust timing cannot exactly reproduce the browser's `PerformanceResourceTiming` boundaries or browser-only TCP connection calibration, so results are methodology-compatible rather than numerically identical.

See [installation details](docs/INSTALLATION.md) and [measurement compatibility](docs/COMPATIBILITY.md).

## License

Licensed under the [MIT License](LICENSE).
