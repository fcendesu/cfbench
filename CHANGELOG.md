# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/fcendesu/cfbench/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/fcendesu/cfbench/releases/tag/v0.1.0
