# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.0] - 2026-08-15

### Added

- Interactive terminal runs now show compact, single-line live telemetry for
  provisional transfer throughput and current-request percentage, running
  latency/jitter, and direction-local loaded latency; final reductions remain
  authoritative. `--verbose` retains permanent per-request lines and
  automation output remains unchanged.
- GitHub Releases now provide verified standalone binaries for Linux x86_64,
  Linux ARM64, macOS Apple Silicon, macOS Intel, and Windows x86_64.
- The checksum-verifying Linux installer now supports ARM64 using the
  standalone release archive.

## [0.3.2] - 2026-08-05

### Compatibility

- Updated the declared Cloudflare Speedtest baseline to v1.13.0, commit
  `5954dee4cc83548a9e5031140df4548f71cd1458`.
- Confirmed that v1.13.0 leaves the request schedule, timing formulas,
  thresholds, reductions, and loaded-latency behavior unchanged.
- Documented upstream's optional `authorizationToken` customer-attribution
  capability, which cfbench does not expose or send.

### Testing

- Updated the pinned offline conformance fixture to v1.13.0 while preserving
  every measurement schedule entry and numeric compatibility vector.

## [0.3.1] - 2026-07-31

### Fixed

- Idle latency and jitter now use every successful v1.12.1 latency phase instead
  of replacing earlier phases with the latest pair of probes.
- Cloudflare server processing time now follows the v1.12.1
  `cfRequestDuration` and `cfSpeed*` selection rules.
- Upload bandwidth, loaded-latency eligibility, and adaptive stopping now use
  Cloudflare's TTFB-based upload duration.

### Testing

- Added a pinned offline Cloudflare Speedtest v1.12.1 conformance fixture for
  the measurement schedule, constants, parsing, accumulation, and reductions.

## [0.3.0] - 2026-07-29

### Changed

- `--quiet` now suppresses normal result output and communicates successful
  completion through the process exit status alone; terminal errors remain on
  stderr.
- `--quiet` now conflicts with `--json` and `--verbose`.
- Usable partial measurements now exit `3`, reserving exit `2` for invalid
  command-line usage reported by Clap.

### Compatibility

- Default text, verbose progress, schema-v1 JSON, and Cloudflare Speedtest
  v1.12.1 measurement behavior are unchanged.

## [0.2.0] - 2026-07-28

### Added

- `--verbose` enables live per-request progress while the default text mode remains concise.
- `--rpki-check` adds an informational, post-measurement invalid-route reachability diagnostic without changing timing, usage, reductions, or exit status.
- Text and schema-v1 JSON output include the requested RPKI diagnostic result.
- `--version` identifies the Cloudflare Speedtest version and commit used as the compatibility baseline.

### Changed

- Complete measurements exit `0`, usable partial measurements exit `2`, and runs without a usable measurement or cancelled runs exit `1`.
- Live Cloudflare integration tests remain ignored by default, with an explicit opt-in command documented for maintainers.

## [0.1.2] - 2026-07-28

### Fixed

- The Debian package installer now grants APT's sandboxed downloader read access to the verified local package, avoiding an unnecessary unsandboxed-download warning.

### Changed

- The one-line installer now reports its download, verification, and installation stages on stderr.

## [0.1.1] - 2026-07-26

### Added

- The Linux installer selects and verifies Debian or RPM release packages automatically when supported by the host distribution.
- `CLAUDE.md` links to the repository agent guidance.
- Installer-routing coverage runs in Linux CI.

### Changed

- Updated actions/checkout, clap, tokio, tokio-util, serde, and serde_json.
- Future weekly Dependabot updates are grouped by ecosystem to reduce duplicate CI runs.
- Installation documentation now uses the one-line package-aware installer.

## [0.1.0] - 2026-07-26

### Added

- Native Cloudflare-compatible latency, bandwidth, jitter, and loaded-latency measurements.
- Human-readable and schema-v1 JSON output.
- IPv4-only and IPv6-only modes, metadata privacy opt-out, timeouts, and quiet mode.
- Debian and RPM packages plus a checksum-verifying Linux installer.

### Changed

- Compatibility baseline is Cloudflare Speedtest v1.12.1.

### Security

- The installer validates the selected release artifact against its published SHA-256 checksum before installation.

[Unreleased]: https://github.com/fcendesu/cfbench/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/fcendesu/cfbench/compare/v0.3.2...v0.4.0
[0.3.2]: https://github.com/fcendesu/cfbench/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/fcendesu/cfbench/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/fcendesu/cfbench/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/fcendesu/cfbench/compare/v0.1.2...v0.2.0
[0.1.2]: https://github.com/fcendesu/cfbench/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/fcendesu/cfbench/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/fcendesu/cfbench/releases/tag/v0.1.0
