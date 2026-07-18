# AGENTS.md

This file contains repository-level instructions for coding agents working on `cfbench`.

## Project summary

`cfbench` is an unofficial native Rust CLI that measures Internet connection quality against Cloudflare's speed-test endpoints. The MVP must reproduce Cloudflare Speedtest's public measurement plan, thresholds, reductions, and result semantics as faithfully as a native HTTP client permits.

The project is a plain command-line program. Do not introduce a TUI, browser runtime, hosted service, or unrelated provider abstraction into the MVP.

## Read before changing code

Read these documents in this order:

1. [`docs/PRD.md`](docs/PRD.md) — product requirements and non-goals.
2. [`docs/MVP.md`](docs/MVP.md) — release boundary and acceptance criteria.
3. [`docs/MEASUREMENT_COMPATIBILITY.md`](docs/MEASUREMENT_COMPATIBILITY.md) — source of truth for measurement behavior.
4. [`docs/superpowers/specs/2026-07-19-cfbench-design.md`](docs/superpowers/specs/2026-07-19-cfbench-design.md) — architecture and module boundaries.
5. [`docs/TEST_STRATEGY.md`](docs/TEST_STRATEGY.md) — required validation and CI gates.
6. [`docs/adr/0001-http-client-reqwest.md`](docs/adr/0001-http-client-reqwest.md) — accepted HTTP-client decision.

When documents disagree, use this precedence:

1. Measurement compatibility specification for algorithms, constants, endpoints, and schedule behavior.
2. MVP scope for what belongs in `0.1.0`.
3. PRD for product behavior.
4. Architecture specification for internal structure.
5. ADRs for accepted technical decisions.

Do not silently resolve a material contradiction. Update the relevant document in the same change and explain the decision.

## Locked MVP decisions

- Language: Rust.
- Binary name: `cfbench`.
- Async runtime: Tokio.
- HTTP client: reqwest with rustls.
- CLI parser: Clap.
- Serialization: Serde and `serde_json`.
- Error types: `thiserror`.
- Interface: ordinary line-oriented CLI output; no TUI.
- Production transport: Cloudflare `__down` and `__up` endpoints.
- Output modes: human-readable text and versioned JSON.
- Packet loss: unavailable/null in the MVP; do not substitute ICMP loss.
- HTTP retries: disabled for measurement requests.
- Tower middleware: not allowed in the timing path.
- HTTP/3: not an MVP requirement.

Changing a locked decision requires a new or amended ADR and corresponding documentation updates.

## Compatibility baseline

The initial documented baseline is Cloudflare Speedtest `v1.11.0`, released July 1, 2026.

Before implementing or changing compatibility-sensitive behavior:

1. Inspect the matching upstream tag or commit.
2. Record the upstream reference used in the change or test fixture.
3. Compare the measurement plan, constants, percentile logic, server-timing parsing, and loaded-latency behavior.
4. Update `docs/MEASUREMENT_COMPATIBILITY.md` when upstream behavior has changed.

Never describe `cfbench` as numerically identical to the browser test. Use terms such as **Cloudflare-compatible methodology** or **faithful native implementation**. Browser `PerformanceResourceTiming` and native reqwest timing observe different boundaries.

## Required measurement behavior

Preserve the published order represented in the compatibility specification. Do not reorder measurements merely for code simplicity.

Important invariants:

- Use `std::time::Instant` for elapsed measurements.
- Reuse one configured HTTP client and connection pool per run.
- Send `Accept-Encoding: identity` and disable transparent response decompression.
- Do not retry a failed measurement automatically.
- Stream download bodies; never concatenate a large response in memory.
- Generate or stream upload bodies without allocating a fresh payload-sized buffer for every request.
- Read and count the actual transferred payload bytes.
- Treat IPv4-only and IPv6-only modes as strict requirements.
- Keep download and upload finish state independent.
- Preserve the initial 100 KB download's finish-gate bypass.
- Exclude ineligible bandwidth points according to the documented duration threshold.
- Keep unloaded, download-loaded, and upload-loaded latency points separate.
- Cancel and await all probe tasks when a transfer or run ends.
- Represent unsupported or unavailable values as `None`/`null`, never as zero or a fabricated estimate.

Any approximation must be explicit in code comments, JSON metadata where relevant, and the compatibility document.

## Architecture boundaries

Keep network I/O, orchestration, statistics, and rendering separate.

Expected responsibilities:

- `cli`: Clap definitions and argument validation only.
- `config`: validated run configuration.
- `plan`: immutable/versioned measurement plan and filtering of disabled stages.
- `transport`: reqwest-backed network operations and response timing observations.
- `measurement`: conversion of transport observations into raw points.
- `runner`: ordered execution, adaptive stopping, cancellation, and partial-result collection.
- `statistics`: pure percentile and jitter logic with no reqwest or terminal dependencies.
- `results`: raw and summarized result models.
- `output`: text and JSON rendering only.

Do not leak reqwest response types into statistics or serialized result models. Keep the transport replaceable if lower-level timing becomes necessary later.

Prefer small modules with a single clear responsibility. Avoid a large `main.rs` or a runner that also parses arguments, calculates statistics, and prints output.

## CLI and output contract

The MVP interface is defined in `docs/PRD.md`. Preserve these rules:

- `cfbench --json` writes exactly one JSON document to stdout.
- Progress and diagnostics go to stderr.
- `--quiet` suppresses progress, not final results or fatal errors.
- `--ipv4` and `--ipv6` are mutually exclusive.
- No alternate screen, cursor-addressed rendering, interactive widgets, or ANSI animation is required.
- Text output uses decimal Mbps and clearly defined byte units.
- JSON uses a schema/version field and stable field names.
- Missing measurements serialize as `null`.
- Raw points contain enough information to reproduce summary reductions.

Treat changes to public CLI flags or JSON field names as compatibility changes. Add tests and document them.

## Error handling and cancellation

- Use typed errors with contextual variants.
- Do not use `unwrap`, `expect`, or `panic!` in normal runtime paths.
- A malformed optional `Server-Timing` value should use the documented fallback and may record a diagnostic; it should not crash the run.
- Timeouts must distinguish request/header/body-stage failures where practical.
- Ctrl+C must cancel active transfers and loaded-latency probes.
- Do not leave detached Tokio tasks running after result rendering.
- Preserve completed measurement points when a later stage fails or the run is cancelled.
- Exit non-zero for failed or cancelled runs, while keeping machine-readable partial-result behavior consistent with the documented contract.

## Rust quality rules

- Use stable Rust unless the repository explicitly declares otherwise.
- Keep `rustfmt` output unchanged.
- Treat all Clippy warnings as errors in CI.
- Prefer explicit domain types over loosely related tuples or maps.
- Use integer byte and bps values where feasible; use floating point for duration and percentile calculations only where needed.
- Reject or filter non-finite floating-point values before reduction or serialization.
- Avoid unnecessary cloning in timing-sensitive paths.
- Do not hold locks across `.await`.
- Bound channels and retained measurement collections where possible.
- Add dependencies only when they materially reduce complexity or improve correctness.
- Keep default features narrow, especially for reqwest and Tokio.

## Testing requirements

Use test-driven development for compatibility-sensitive behavior. A bug fix must include a regression test that fails without the fix.

Required local checks before declaring a change complete:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
cargo build --release
```

If the Rust project has not yet been scaffolded, state that these commands are not yet applicable and validate the documentation or generated files directly instead.

Testing priorities:

1. Pure deterministic tests for percentile, jitter, eligibility, and scheduling.
2. Local HTTP integration tests for streaming, delays, malformed headers, failures, and timeouts.
3. CLI tests for stdout/stderr separation, flags, exit codes, and JSON parsing.
4. Ignored live tests against Cloudflare for broad invariants only.

Do not make ordinary CI depend on external Internet access or on a live speed test. Live tests consume data and can be unstable.

For timing tests, prefer paused Tokio time, controlled channels, or a local fixture server over real sleeps. Tests must not assert unrealistically tight wall-clock durations on shared CI runners.

## Memory and performance guardrails

- A 250 MB download must not require a 250 MB in-memory response buffer.
- Repeated uploads must not retain previous payload-sized allocations.
- Terminal progress must not be printed per network chunk.
- The client must remain capable of measuring common gigabit connections without local CPU, allocation, logging, or serialization becoming the obvious bottleneck.
- Performance claims require a reproducible local benchmark or measurement, not intuition.

## Documentation requirements

Update documentation in the same change when modifying:

- measurement constants or schedule;
- compatibility claims;
- CLI flags or output fields;
- module boundaries;
- accepted dependencies or architectural decisions;
- supported platforms or release requirements.

Add an ADR for a durable architectural decision. Do not rewrite an accepted ADR as though the original choice never existed; supersede it and link the replacement.

Keep examples consistent with the actual binary behavior. Avoid unresolved placeholder markers in release-ready documentation unless the item is explicitly tracked and blocks release.

## Scope control

Do not add these to MVP work unless the user explicitly expands scope and the documents are updated first:

- TURN/WebRTC packet loss;
- AIM scores;
- CSV export;
- historical storage or dashboards;
- custom providers;
- custom measurement-plan files;
- result sharing or hosted APIs;
- background scheduling or daemon mode;
- full-screen TUI;
- browser embedding;
- manual Cloudflare data-center selection.

When an attractive feature is outside the MVP, document it as a possible follow-up rather than implementing it opportunistically.

## Agent workflow

For each task:

1. Read the relevant documents and existing tests.
2. Identify whether the change is compatibility-sensitive, public-contract-sensitive, or internal-only.
3. Make the smallest coherent change that satisfies the task.
4. Add or update tests before or alongside implementation.
5. Run the full applicable validation commands.
6. Review the diff for accidental scope expansion, large allocations, retries, output contamination, and unsupported compatibility claims.
7. Report exactly what changed, the commands run, and any known limitation.

Do not claim a test, build, benchmark, or compatibility result unless the corresponding command or comparison was run in the current work session and its output was inspected.

## Commit guidance

Use focused commits. Suggested prefixes:

- `feat:` user-visible functionality;
- `fix:` defect correction;
- `test:` test-only changes;
- `docs:` documentation;
- `refactor:` behavior-preserving internal changes;
- `perf:` measured performance improvements;
- `ci:` automation and release workflows;
- `chore:` maintenance that does not fit another category.

Do not mix broad refactors with compatibility changes unless the refactor is necessary for the change and separately reviewable.

## Definition of done

A task is complete only when:

- behavior matches the applicable requirements and compatibility rules;
- tests cover the changed behavior;
- all applicable quality gates pass;
- stdout/stderr and JSON contracts remain valid;
- no timing tasks or streams are leaked;
- documentation is updated when required;
- limitations are stated honestly.
