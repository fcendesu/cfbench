# Contributing to cfbench

Thank you for helping improve cfbench. Bug reports, compatibility findings,
documentation fixes, and focused code changes are welcome.

## Before you start

- Search existing issues and pull requests before opening a duplicate.
- Use an issue for behavior changes that need design discussion.
- Keep changes focused on cfbench as a native, line-oriented Rust CLI for
  Cloudflare's speed-test endpoints.
- Do not include real public IP addresses, ISP details, ASN data, or copied live
  results in examples, fixtures, tests, or documentation. Use reserved example
  addresses and clearly synthetic network names.

For security vulnerabilities, follow [SECURITY.md](SECURITY.md) instead of
opening a public issue.

## Development setup

Rust 1.95 or newer is required.

```bash
git clone https://github.com/fcendesu/cfbench.git
cd cfbench
cargo build
```

Create a branch from the latest `main` and keep commits focused. Conventional
Commit subjects are preferred, for example:

```text
fix(output): preserve progress on probe failure
docs(readme): clarify JSON output
```

## Project constraints

- Reuse one configured HTTP client per run.
- Do not add automatic retries or response decompression to measurement traffic.
- Stream transfer bodies instead of buffering data proportional to payload size.
- Keep transport, orchestration, statistics, result models, and rendering
  separate.
- Preserve the one-document stdout contract for `--json`; progress and
  diagnostics belong on stderr.
- Do not add packet-loss substitutes, HTTP/3, TURN/WebRTC, a browser runtime,
  daemon, dashboard, or TUI without an explicit scope decision.

Read [docs/COMPATIBILITY.md](docs/COMPATIBILITY.md) before changing request
schedules, timing boundaries, thresholds, or statistical reductions. Native
results must not be described as numerically identical to Cloudflare's browser
test.

The scheduled `Cloudflare compatibility` workflow is a review trigger, not an
automatic compatibility claim. When it opens a tracking issue, inspect the
linked release, commits, and methodology-sensitive paths; update the pinned
fixture, implementation, documentation, and version metadata together when
required. Maintainers can also run the workflow manually from GitHub Actions.
Keep its tests offline and do not execute code downloaded from upstream.

## Testing

Run the full local validation suite before submitting a code change:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
cargo build --release
python3 -m unittest tests/test_cloudflare_compatibility_monitor.py
```

Live Cloudflare tests consume network traffic and are ignored by default. They
must not be required by CI. Run them manually only when the change requires it:

```bash
cargo test --test live_cloudflare -- --ignored
```

## Pull requests

- Explain what changed and why.
- Add or update tests for behavioral changes.
- Update user documentation and `CHANGELOG.md` when the change is user-visible.
- Keep generated files, build output, local development notes, and unrelated
  refactors out of the pull request.
- Ensure required CI checks pass. Maintainers may request revisions before
  merging.

By participating, you agree to follow the
[Code of Conduct](CODE_OF_CONDUCT.md).
