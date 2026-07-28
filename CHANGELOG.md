# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/fcendesu/cfbench/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/fcendesu/cfbench/compare/v0.1.2...v0.2.0
[0.1.2]: https://github.com/fcendesu/cfbench/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/fcendesu/cfbench/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/fcendesu/cfbench/releases/tag/v0.1.0
