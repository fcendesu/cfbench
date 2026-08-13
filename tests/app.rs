use std::future::Future;
use std::io::Write;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use cfbench::app::{
    AppError, EXIT_COMPLETE, EXIT_FAILURE, EXIT_PARTIAL, OutputOptions, ProgressMode, exit_status,
    run_with_signal, run_with_signal_and_progress,
    run_with_signal_and_progress_with_compact_draw_target, spawn_compact_progress_renderer,
    spawn_progress_renderer, write_outcome, write_progress,
};
use cfbench::cancellation::CancellationToken;
use cfbench::error::TransportError;
use cfbench::measurement::TimingObservation;
use cfbench::plan::{MeasurementPlan, MeasurementStep};
use cfbench::progress::{ProgressEvent, ProgressReporter, ProgressStage, TransferTelemetry};
use cfbench::results::{EdgeLocation, LatencyPoint, MetadataStatus, NetworkMetadata, RunResult};
use cfbench::runner::{MeasurementFuture, MeasurementTransport, RunOutcome, Runner, RunnerError};

#[test]
fn accepted_core_points_with_a_core_failure_are_partial() {
    let mut outcome = successful_latency_outcome();
    outcome.error = Some(RunnerError::Transport {
        stage: "download".into(),
        source: fixture_error(),
    });

    assert_eq!(exit_status(&outcome), EXIT_PARTIAL);
}

#[test]
fn metadata_and_rpki_diagnostics_do_not_downgrade_complete() {
    let mut outcome = successful_latency_outcome();
    outcome
        .result
        .diagnostics
        .push("metadata collection failed".into());

    assert_eq!(exit_status(&outcome), EXIT_COMPLETE);
}

#[test]
fn cancellation_is_failure_even_after_an_accepted_point() {
    let mut outcome = successful_latency_outcome();
    outcome.error = Some(RunnerError::Cancelled {
        stage: "run".into(),
    });

    assert_eq!(exit_status(&outcome), EXIT_FAILURE);
}

#[test]
fn output_options_select_progress_mode_from_flags_and_terminal_state() {
    let cases = [
        (
            OutputOptions {
                json: false,
                quiet: false,
                verbose: false,
            },
            true,
            ProgressMode::Compact,
        ),
        (
            OutputOptions {
                json: false,
                quiet: false,
                verbose: false,
            },
            false,
            ProgressMode::Disabled,
        ),
        (
            OutputOptions {
                json: false,
                quiet: false,
                verbose: true,
            },
            true,
            ProgressMode::Verbose,
        ),
        (
            OutputOptions {
                json: false,
                quiet: false,
                verbose: true,
            },
            false,
            ProgressMode::Verbose,
        ),
        (
            OutputOptions {
                json: true,
                quiet: false,
                verbose: false,
            },
            true,
            ProgressMode::Disabled,
        ),
        (
            OutputOptions {
                json: false,
                quiet: true,
                verbose: false,
            },
            true,
            ProgressMode::Disabled,
        ),
    ];

    for (options, terminal, expected) in cases {
        assert_eq!(options.progress_mode(terminal), expected);
    }
}

#[tokio::test]
async fn verbose_mode_preserves_the_legacy_progress_transcript() {
    let output = run_progress_fixture(ProgressMode::Verbose).await;
    assert_eq!(
        output.stderr,
        concat!(
            "Testing against Cloudflare edge...\n",
            "[latency 1/2] 10.00 ms\n",
            "[latency 2/2] 10.00 ms\n",
        )
    );
}

#[tokio::test]
async fn disabled_mode_writes_no_progress() {
    let output = run_progress_fixture(ProgressMode::Disabled).await;
    assert!(!output.stderr.contains("Testing against Cloudflare edge"));
    assert!(!output.stderr.contains("latency"));
}

#[tokio::test]
async fn compact_renderer_shutdown_does_not_change_results_or_status() {
    let output = run_progress_fixture(ProgressMode::Compact).await;
    assert_eq!(output.latency_points, 2);
    assert_eq!(output.status, EXIT_COMPLETE);
    assert!(!output.stderr.contains("latency"));
}

#[tokio::test]
async fn compact_renderer_panic_does_not_change_outcome_or_exit_status() {
    let options = OutputOptions {
        json: false,
        quiet: false,
        verbose: false,
    };
    let runner = Runner::new(
        ImmediateLatencyTransport,
        MeasurementPlan {
            upstream_version: "test",
            upstream_commit: "test",
            steps: vec![MeasurementStep::Latency { packets: 2 }],
        },
    )
    .with_loaded_latency(false);
    let renderer_panicked = Arc::new(AtomicBool::new(false));

    let run = run_with_signal_and_progress_with_compact_draw_target(
        &runner,
        std::future::pending::<std::io::Result<()>>(),
        options,
        SharedWriter::default(),
        indicatif::ProgressDrawTarget::term_like(Box::new(RecordingTerm::panicking(
            renderer_panicked.clone(),
        ))),
    )
    .await;

    assert!(renderer_panicked.load(Ordering::SeqCst));
    assert!(run.progress_error.is_none());
    assert!(run.outcome.error.is_none());
    assert_eq!(run.outcome.result.raw.latency.len(), 2);
    assert_eq!(exit_status(&run.outcome), EXIT_COMPLETE);
}

#[tokio::test]
async fn compact_telemetry_renderer_draw_failure_does_not_change_outcome_or_exit_status() {
    let options = OutputOptions {
        json: false,
        quiet: false,
        verbose: false,
    };
    let runner = Runner::new(
        ImmediateLatencyTransport,
        MeasurementPlan {
            upstream_version: "test",
            upstream_commit: "test",
            steps: vec![MeasurementStep::Latency { packets: 2 }],
        },
    )
    .with_loaded_latency(false);
    let draw_failed = Arc::new(AtomicBool::new(false));

    let run = run_with_signal_and_progress_with_compact_draw_target(
        &runner,
        std::future::pending::<std::io::Result<()>>(),
        options,
        SharedWriter::default(),
        indicatif::ProgressDrawTarget::term_like(Box::new(RecordingTerm::failing(
            draw_failed.clone(),
        ))),
    )
    .await;

    assert!(draw_failed.load(Ordering::SeqCst));
    assert!(run.progress_error.is_none());
    assert!(run.outcome.error.is_none());
    assert_eq!(run.outcome.result.raw.latency.len(), 2);
    assert_eq!(exit_status(&run.outcome), EXIT_COMPLETE);
}

#[test]
fn compact_telemetry_renderer_replaces_one_status_and_clears_on_channel_closure() {
    let terminal = RecordingTerm::default();
    let operations = terminal.operations.clone();
    let (reporter, receiver) = ProgressReporter::channel(2);
    let renderer = spawn_compact_progress_renderer(
        receiver,
        indicatif::ProgressDrawTarget::term_like(Box::new(terminal)),
    )
    .unwrap();

    reporter.emit(ProgressEvent::RequestStarted {
        stage: ProgressStage::Transfer {
            direction: cfbench::plan::Direction::Download,
            requested_bytes: 100_000_000,
        },
        current: Some(1),
        total: Some(3),
    });
    reporter.emit(ProgressEvent::TransferAdvanced {
        direction: cfbench::plan::Direction::Download,
        requested_bytes: 100_000_000,
        current: 1,
        total: 3,
        transferred_bytes: 63_000_000,
        window_bytes: 20_062_500,
        window_duration_ms: 250.0,
    });
    drop(reporter);
    renderer.join().unwrap();

    let operations = operations.lock().unwrap();
    assert!(operations.iter().any(|operation| {
        matches!(operation, TerminalOperation::Draw(text) if text.contains("Download 100 MB 1/3 · 642 Mbps · 63%"))
    }));
    assert_eq!(operations.last(), Some(&TerminalOperation::Clear));
    assert!(operations.iter().all(
        |operation| !matches!(operation, TerminalOperation::Draw(text) if text.contains('\n'))
    ));
}

#[test]
fn verbose_telemetry_renderer_ignores_snapshots_and_keeps_completion_only_transcript() {
    let writer = SharedWriter::default();
    let inspection = writer.clone();
    let cancellation = CancellationToken::new();
    let (reporter, receiver) = ProgressReporter::channel(3);
    let renderer = spawn_progress_renderer(receiver, writer, cancellation.clone()).unwrap();

    reporter.emit(ProgressEvent::RequestStarted {
        stage: ProgressStage::Transfer {
            direction: cfbench::plan::Direction::Download,
            requested_bytes: 100_000_000,
        },
        current: Some(1),
        total: Some(3),
    });
    reporter.emit(ProgressEvent::TransferAdvanced {
        direction: cfbench::plan::Direction::Download,
        requested_bytes: 100_000_000,
        current: 1,
        total: 3,
        transferred_bytes: 63_000_000,
        window_bytes: 20_062_500,
        window_duration_ms: 250.0,
    });
    reporter.emit(ProgressEvent::TransferCompleted {
        direction: cfbench::plan::Direction::Download,
        requested_bytes: 100_000_000,
        current: 1,
        total: 3,
        bps: 676_870_000,
        adjusted_duration_ms: 1_188.4,
    });
    drop(reporter);

    renderer.join().unwrap().unwrap();
    assert_eq!(
        inspection.text(),
        concat!(
            "Testing against Cloudflare edge...\n",
            "[download 100 MB 1/3] 676.87 Mbps — 1.19 s\n",
        )
    );
    assert_eq!(inspection.flushes(), 2);
    assert!(!cancellation.is_cancelled());
}

#[test]
fn telemetry_stays_silent_in_disabled_json_quiet_and_redirected_modes() {
    let event = ProgressEvent::TransferAdvanced {
        direction: cfbench::plan::Direction::Upload,
        requested_bytes: 50_000_000,
        current: 1,
        total: 3,
        transferred_bytes: 25_000_000,
        window_bytes: 10_000_000,
        window_duration_ms: 250.0,
    };
    let cases = [
        (
            OutputOptions {
                json: true,
                quiet: false,
                verbose: false,
            },
            true,
        ),
        (
            OutputOptions {
                json: false,
                quiet: true,
                verbose: false,
            },
            true,
        ),
        (
            OutputOptions {
                json: false,
                quiet: false,
                verbose: false,
            },
            false,
        ),
    ];

    for (options, terminal) in cases {
        let mode = options.progress_mode(terminal);
        assert_eq!(mode, ProgressMode::Disabled);
        let mut stderr = SharedWriter::default();
        write_progress(mode, &mut stderr).unwrap();
        ProgressReporter::disabled().emit(event.clone());
        assert!(stderr.text().is_empty());
        assert_eq!(stderr.flushes(), 0);
    }
}

#[test]
fn progress_renderer_writes_and_flushes_each_line_then_joins_on_channel_closure() {
    let writer = SharedWriter::default();
    let inspection = writer.clone();
    let cancellation = CancellationToken::new();
    let (reporter, receiver) = ProgressReporter::channel(1);
    let renderer = spawn_progress_renderer(receiver, writer, cancellation.clone()).unwrap();

    reporter.emit(ProgressEvent::LatencyCompleted {
        current: 1,
        total: 1,
        latency_ms: 12.5,
    });
    drop(reporter);

    renderer.join().unwrap().unwrap();
    assert_eq!(
        inspection.text(),
        concat!(
            "Testing against Cloudflare edge...\n",
            "[latency 1/1] 12.50 ms\n"
        )
    );
    assert_eq!(inspection.flushes(), 2);
    assert!(!cancellation.is_cancelled());
}

#[test]
fn opening_progress_line_flushes_and_respects_suppression() {
    let mut text = SharedWriter::default();
    write_progress(ProgressMode::Verbose, &mut text).unwrap();
    assert_eq!(text.text(), "Testing against Cloudflare edge...\n");
    assert_eq!(text.flushes(), 1);

    for mode in [ProgressMode::Disabled, ProgressMode::Compact] {
        let mut suppressed = SharedWriter::default();
        write_progress(mode, &mut suppressed).unwrap();
        assert!(suppressed.text().is_empty());
        assert_eq!(suppressed.flushes(), 0);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn renderer_write_failure_cancels_runner_and_is_retained_after_join() {
    let lifecycle = Arc::new(FailureLifecycle::default());
    let runner = Runner::new(
        CancellationAwareTransport(lifecycle.clone()),
        MeasurementPlan {
            upstream_version: "test",
            upstream_commit: "test",
            steps: vec![MeasurementStep::Latency { packets: 1 }],
        },
    )
    .with_loaded_latency(false);

    let run = tokio::time::timeout(
        Duration::from_secs(1),
        run_with_signal_and_progress(
            &runner,
            std::future::pending::<std::io::Result<()>>(),
            OutputOptions {
                json: false,
                quiet: false,
                verbose: true,
            },
            ProgressMode::Verbose,
            BlockingFailureWriter(lifecycle.clone()),
        ),
    )
    .await
    .expect("progress failure must not deadlock the runner");

    assert!(matches!(run.progress_error, Some(AppError::Write(_))));
    assert!(matches!(
        run.outcome.error,
        Some(RunnerError::Cancelled { ref stage }) if stage == "latency"
    ));
    assert!(lifecycle.transport_finished.load(Ordering::SeqCst));
    assert!(lifecycle.writer_dropped.load(Ordering::SeqCst));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn renderer_failure_after_channel_fills_does_not_deadlock_or_change_results() {
    const PACKETS: u32 = 300;

    let lifecycle = Arc::new(ChannelFillLifecycle::default());
    let runner = Runner::new(
        CountingLatencyTransport {
            lifecycle: lifecycle.clone(),
            expected: PACKETS,
        },
        MeasurementPlan {
            upstream_version: "test",
            upstream_commit: "test",
            steps: vec![MeasurementStep::Latency { packets: PACKETS }],
        },
    )
    .with_loaded_latency(false);

    let run = tokio::time::timeout(
        Duration::from_secs(1),
        run_with_signal_and_progress(
            &runner,
            std::future::pending::<std::io::Result<()>>(),
            OutputOptions {
                json: false,
                quiet: false,
                verbose: true,
            },
            ProgressMode::Verbose,
            FailAfterAllRequestsWriter(lifecycle),
        ),
    )
    .await
    .expect("a full progress channel and writer failure must not deadlock");

    assert!(matches!(run.progress_error, Some(AppError::Write(_))));
    match run.outcome.error.as_ref() {
        Some(RunnerError::Cancelled { stage }) => {
            assert_eq!(stage, "run");
            assert_eq!(run.outcome.result.failures.len(), 1);
        }
        None => assert!(run.outcome.result.failures.is_empty()),
        Some(error) => panic!("unexpected renderer-cancellation outcome: {error}"),
    }
    assert_eq!(run.outcome.result.raw.latency.len(), PACKETS as usize);
}

#[test]
fn json_mode_writes_one_partial_document_without_progress_then_returns_one() {
    let options = OutputOptions {
        json: true,
        quiet: false,
        verbose: false,
    };
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    write_progress(ProgressMode::Disabled, &mut stderr).unwrap();
    let exit = write_outcome(partial_failure(), options, &mut stdout, &mut stderr).unwrap();

    assert_eq!(exit, 1);
    let parsed: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
    assert_eq!(parsed["schema_version"], 1);
    assert!(parsed["summary"]["download_bps"].is_null());
    assert!(
        !String::from_utf8(stdout)
            .unwrap()
            .contains("Testing against")
    );
    let stderr = String::from_utf8(stderr).unwrap();
    assert!(!stderr.contains("Testing against"));
    assert!(stderr.contains("error: measurement cancelled"));
}

#[test]
fn json_mode_keeps_additive_metadata_in_one_schema_v1_document() {
    let mut result = RunResult::empty();
    result.started_at = "2026-07-19T09:02:59.123Z".to_owned();
    result.target.metadata_status = MetadataStatus::Available;
    result.target.metadata = Some(NetworkMetadata {
        public_ip: Some("2001:db8::1".to_owned()),
        asn: Some(64_496),
        edge: EdgeLocation {
            colo: Some("XYZ".to_owned()),
            ..EdgeLocation::default()
        },
        ..NetworkMetadata::default()
    });
    result.raw.latency.push(LatencyPoint {
        ping_ms: 10.0,
        ttfb_ms: 20.0,
        server_time_ms: 0.0,
        http_version: Some("HTTP/2".to_owned()),
        measured_at_unix_ms: 0,
    });
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let exit = write_outcome(
        RunOutcome {
            result,
            error: None,
        },
        OutputOptions {
            json: true,
            quiet: false,
            verbose: false,
        },
        &mut stdout,
        &mut stderr,
    )
    .unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&stdout).unwrap();

    assert_eq!(exit, 0);
    assert_eq!(parsed["schema_version"], 1);
    assert_eq!(parsed["target"]["metadata_status"], "available");
    assert_eq!(parsed["target"]["metadata"]["public_ip"], "2001:db8::1");
    assert!(stderr.is_empty());
}

#[test]
fn quiet_complete_run_is_fully_silent() {
    let mut outcome = successful_latency_outcome();
    outcome
        .result
        .diagnostics
        .push("suppressed informational diagnostic".to_owned());
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit = write_outcome(
        outcome,
        OutputOptions {
            json: false,
            quiet: true,
            verbose: false,
        },
        &mut stdout,
        &mut stderr,
    )
    .unwrap();

    assert_eq!(exit, EXIT_COMPLETE);
    assert!(stdout.is_empty());
    assert!(stderr.is_empty());
}

#[test]
fn quiet_partial_run_writes_only_terminal_error_and_exits_three() {
    let mut outcome = partial_transport_failure();
    outcome
        .result
        .diagnostics
        .push("suppressed retained-result detail".to_owned());
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let exit = write_outcome(
        outcome,
        OutputOptions {
            json: false,
            quiet: true,
            verbose: false,
        },
        &mut stdout,
        &mut stderr,
    )
    .unwrap();

    assert_eq!(exit, 3);
    assert!(stdout.is_empty());
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        "error: transport failed during download: endpoint https://fixture.invalid/__down returned HTTP status 503\n"
    );
}

#[test]
fn quiet_terminal_cancellation_writes_only_terminal_error_and_exits_one() {
    let mut outcome = partial_failure();
    outcome
        .result
        .diagnostics
        .push("suppressed cancellation detail".to_owned());
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let exit = write_outcome(
        outcome,
        OutputOptions {
            json: false,
            quiet: true,
            verbose: false,
        },
        &mut stdout,
        &mut stderr,
    )
    .unwrap();

    assert_eq!(exit, EXIT_FAILURE);
    assert!(stdout.is_empty());
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        "error: measurement cancelled during download\n"
    );
}

#[test]
fn quiet_suppresses_success_diagnostics_and_nonquiet_keeps_failure_diagnostics() {
    let options = OutputOptions {
        json: false,
        quiet: true,
        verbose: false,
    };
    let mut success = cfbench::results::RunResult::empty();
    success.raw.latency.push(LatencyPoint {
        ping_ms: 10.0,
        ttfb_ms: 20.0,
        server_time_ms: 0.0,
        http_version: Some("HTTP/2".to_owned()),
        measured_at_unix_ms: 0,
    });
    success.diagnostics.push("successful diagnostic".to_owned());
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit = write_outcome(
        RunOutcome {
            result: success,
            error: None,
        },
        options,
        &mut stdout,
        &mut stderr,
    )
    .unwrap();
    assert_eq!(exit, 0);
    assert!(stderr.is_empty());
    assert!(stdout.is_empty());

    let mut failure = partial_failure();
    failure
        .result
        .diagnostics
        .push("failed diagnostic".to_owned());
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit = write_outcome(
        failure,
        OutputOptions {
            json: false,
            quiet: false,
            verbose: false,
        },
        &mut stdout,
        &mut stderr,
    )
    .unwrap();
    assert_eq!(exit, 1);
    let stderr = String::from_utf8(stderr).unwrap();
    assert!(stderr.contains("diagnostic: failed diagnostic\n"));
    assert!(stderr.contains("error: measurement cancelled"));
    assert!(String::from_utf8(stdout).unwrap().starts_with(concat!(
        "cfbench ",
        env!("CARGO_PKG_VERSION"),
        "\n"
    )));
}

#[tokio::test]
async fn signal_is_polled_before_runner_starts_network_work() {
    let installed = Arc::new(AtomicBool::new(false));
    let runner = Runner::new(
        InstallAwareTransport(installed.clone()),
        MeasurementPlan {
            upstream_version: "test",
            upstream_commit: "test",
            steps: vec![MeasurementStep::Latency { packets: 1 }],
        },
    )
    .with_loaded_latency(false);

    let outcome = run_with_signal(&runner, InstallSignal(installed)).await;

    assert!(outcome.error.is_none());
    assert_eq!(outcome.result.raw.latency.len(), 1);
}

#[tokio::test]
async fn selected_signal_forces_terminal_outcome_after_concurrent_final_success() {
    let final_operation_polled = Arc::new(AtomicBool::new(false));
    let runner = Runner::new(
        ConcurrentSuccessTransport(final_operation_polled.clone()),
        MeasurementPlan {
            upstream_version: "test",
            upstream_commit: "test",
            steps: vec![MeasurementStep::Latency { packets: 1 }],
        },
    )
    .with_loaded_latency(false);

    let outcome = run_with_signal(&runner, ReadyWithFinalOperation(final_operation_polled)).await;

    assert!(matches!(
        outcome.error,
        Some(RunnerError::Cancelled { ref stage }) if stage == "run"
    ));
    assert_eq!(outcome.result.raw.latency.len(), 1);
    assert_eq!(outcome.result.failures.len(), 1);
    assert!(
        outcome
            .result
            .failures
            .iter()
            .any(|failure| failure == "measurement cancelled during run")
    );

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit = write_outcome(
        outcome,
        OutputOptions {
            json: false,
            quiet: true,
            verbose: false,
        },
        &mut stdout,
        &mut stderr,
    )
    .unwrap();
    assert_eq!(exit, 1);
    assert!(stdout.is_empty());
}

#[tokio::test]
async fn selected_signal_supersedes_a_concurrent_terminal_error_and_preserves_history() {
    let final_operation_polled = Arc::new(AtomicBool::new(false));
    let runner = Runner::new(
        ConcurrentErrorTransport(final_operation_polled.clone()),
        MeasurementPlan {
            upstream_version: "test",
            upstream_commit: "test",
            steps: vec![MeasurementStep::Latency { packets: 1 }],
        },
    )
    .with_loaded_latency(false);

    let outcome = run_with_signal(&runner, ReadyWithFinalOperation(final_operation_polled)).await;

    assert!(matches!(
        outcome.error,
        Some(RunnerError::Cancelled { ref stage }) if stage == "run"
    ));
    assert_eq!(outcome.result.failures.len(), 2);
    assert!(outcome.result.failures[0].contains("transport failed during latency"));
    assert_eq!(
        outcome.result.failures[1],
        "measurement cancelled during run"
    );
}

#[tokio::test]
async fn transport_failure_stderr_includes_stage_redacted_endpoint_and_cause() {
    let endpoint = "https://speed.cloudflare.com/__down";
    let runner = Runner::new(
        ErrorTransport(endpoint.to_owned()),
        MeasurementPlan {
            upstream_version: "test",
            upstream_commit: "test",
            steps: vec![MeasurementStep::Latency { packets: 1 }],
        },
    )
    .with_loaded_latency(false);
    let outcome = runner.run(&CancellationToken::new()).await;
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let exit = write_outcome(
        outcome,
        OutputOptions {
            json: false,
            quiet: true,
            verbose: false,
        },
        &mut stdout,
        &mut stderr,
    )
    .unwrap();

    assert_eq!(exit, 1);
    let stderr = String::from_utf8(stderr).unwrap();
    assert!(stderr.contains("during latency"));
    assert!(stderr.contains(endpoint));
    assert!(stderr.contains("HTTP status 503"));
    assert!(!stderr.contains("bytes="));
}

fn partial_failure() -> RunOutcome {
    let error = RunnerError::Cancelled {
        stage: "download".to_owned(),
    };
    let mut result = cfbench::results::RunResult::empty();
    result.failures.push(error.to_string());
    RunOutcome {
        result,
        error: Some(error),
    }
}

fn partial_transport_failure() -> RunOutcome {
    let mut outcome = successful_latency_outcome();
    outcome.error = Some(RunnerError::Transport {
        stage: "download".to_owned(),
        source: fixture_error(),
    });
    outcome.result.failures.push(
        outcome
            .error
            .as_ref()
            .expect("partial failure has terminal error")
            .to_string(),
    );
    outcome
}

fn successful_latency_outcome() -> RunOutcome {
    let mut result = RunResult::empty();
    result.raw.latency.push(LatencyPoint {
        ping_ms: 10.0,
        ttfb_ms: 20.0,
        server_time_ms: 0.0,
        http_version: Some("HTTP/2".to_owned()),
        measured_at_unix_ms: 0,
    });
    RunOutcome {
        result,
        error: None,
    }
}

fn fixture_error() -> TransportError {
    TransportError::HttpStatus {
        endpoint: "https://fixture.invalid/__down".to_owned(),
        status: 503,
        payload_bytes: 0,
    }
}

struct FixtureOutput {
    stderr: String,
    latency_points: usize,
    status: u8,
}

async fn run_progress_fixture(mode: ProgressMode) -> FixtureOutput {
    let options = OutputOptions {
        json: false,
        quiet: false,
        verbose: mode == ProgressMode::Verbose,
    };
    let writer = SharedWriter::default();
    let inspection = writer.clone();
    let runner = Runner::new(
        ImmediateLatencyTransport,
        MeasurementPlan {
            upstream_version: "test",
            upstream_commit: "test",
            steps: vec![MeasurementStep::Latency { packets: 2 }],
        },
    )
    .with_loaded_latency(false);

    let run = run_with_signal_and_progress(
        &runner,
        std::future::pending::<std::io::Result<()>>(),
        options,
        mode,
        writer,
    )
    .await;
    assert!(run.progress_error.is_none());
    let latency_points = run.outcome.result.raw.latency.len();

    let mut stdout = Vec::new();
    let mut final_stderr = inspection.clone();
    let status = write_outcome(run.outcome, options, &mut stdout, &mut final_stderr).unwrap();
    assert_eq!(status, EXIT_COMPLETE);

    FixtureOutput {
        stderr: inspection.text(),
        latency_points,
        status,
    }
}

#[derive(Clone, Debug, Default)]
struct RecordingTerm {
    operations: Arc<Mutex<Vec<TerminalOperation>>>,
    panic_once: Option<Arc<AtomicBool>>,
    fail_once: Option<Arc<AtomicBool>>,
}

#[derive(Debug, Eq, PartialEq)]
enum TerminalOperation {
    Draw(String),
    Clear,
}

impl RecordingTerm {
    fn panicking(panicked: Arc<AtomicBool>) -> Self {
        Self {
            panic_once: Some(panicked),
            ..Self::default()
        }
    }

    fn failing(failed: Arc<AtomicBool>) -> Self {
        Self {
            fail_once: Some(failed),
            ..Self::default()
        }
    }

    fn fail_draw_once(&self) -> std::io::Result<()> {
        if let Some(failed) = &self.fail_once
            && !failed.swap(true, Ordering::SeqCst)
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "scripted compact draw failure",
            ));
        }
        Ok(())
    }
}

impl indicatif::TermLike for RecordingTerm {
    fn width(&self) -> u16 {
        if let Some(panicked) = &self.panic_once
            && !panicked.swap(true, Ordering::SeqCst)
        {
            panic!("scripted compact renderer panic");
        }
        80
    }

    fn move_cursor_up(&self, _: usize) -> std::io::Result<()> {
        Ok(())
    }

    fn move_cursor_down(&self, _: usize) -> std::io::Result<()> {
        Ok(())
    }

    fn move_cursor_right(&self, _: usize) -> std::io::Result<()> {
        Ok(())
    }

    fn move_cursor_left(&self, _: usize) -> std::io::Result<()> {
        Ok(())
    }

    fn write_line(&self, text: &str) -> std::io::Result<()> {
        self.fail_draw_once()?;
        self.operations
            .lock()
            .unwrap()
            .push(TerminalOperation::Draw(format!("{text}\n")));
        Ok(())
    }

    fn write_str(&self, text: &str) -> std::io::Result<()> {
        self.fail_draw_once()?;
        self.operations
            .lock()
            .unwrap()
            .push(TerminalOperation::Draw(text.to_owned()));
        Ok(())
    }

    fn clear_line(&self) -> std::io::Result<()> {
        self.operations
            .lock()
            .unwrap()
            .push(TerminalOperation::Clear);
        Ok(())
    }

    fn flush(&self) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(Clone, Default)]
struct SharedWriter(Arc<Mutex<WriterState>>);

#[derive(Default)]
struct WriterState {
    bytes: Vec<u8>,
    flushes: usize,
}

impl SharedWriter {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().bytes.clone()).unwrap()
    }

    fn flushes(&self) -> usize {
        self.0.lock().unwrap().flushes
    }
}

impl Write for SharedWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().bytes.extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.lock().unwrap().flushes += 1;
        Ok(())
    }
}

struct ImmediateLatencyTransport;

impl MeasurementTransport for ImmediateLatencyTransport {
    fn latency<'a>(&'a self, _: &'a CancellationToken) -> MeasurementFuture<'a> {
        Box::pin(async { Ok(TimingObservation::from_millis(20.0, 20.0, 10.0, 0, "1.1")) })
    }

    fn loaded_latency<'a>(
        &'a self,
        _: cfbench::plan::Direction,
        _: &'a CancellationToken,
    ) -> MeasurementFuture<'a> {
        unused_measurement()
    }

    fn download<'a>(
        &'a self,
        _: u64,
        _: Option<&'a str>,
        _: TransferTelemetry,
        _: &'a CancellationToken,
    ) -> MeasurementFuture<'a> {
        unused_measurement()
    }

    fn upload<'a>(
        &'a self,
        _: u64,
        _: TransferTelemetry,
        _: &'a CancellationToken,
    ) -> MeasurementFuture<'a> {
        unused_measurement()
    }
}

#[derive(Default)]
struct FailureLifecycle {
    transport_started: Mutex<bool>,
    transport_started_notification: Condvar,
    transport_finished: AtomicBool,
    writer_dropped: AtomicBool,
}

struct BlockingFailureWriter(Arc<FailureLifecycle>);

impl Write for BlockingFailureWriter {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        let mut started = self.0.transport_started.lock().unwrap();
        while !*started {
            started = self.0.transport_started_notification.wait(started).unwrap();
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "scripted progress writer failure",
        ))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for BlockingFailureWriter {
    fn drop(&mut self) {
        self.0.writer_dropped.store(true, Ordering::SeqCst);
    }
}

struct CancellationAwareTransport(Arc<FailureLifecycle>);

impl MeasurementTransport for CancellationAwareTransport {
    fn latency<'a>(&'a self, cancellation: &'a CancellationToken) -> MeasurementFuture<'a> {
        let lifecycle = self.0.clone();
        Box::pin(async move {
            {
                let mut started = lifecycle.transport_started.lock().unwrap();
                *started = true;
                lifecycle.transport_started_notification.notify_one();
            }
            cancellation.cancelled().await;
            lifecycle.transport_finished.store(true, Ordering::SeqCst);
            Err(TransportError::Cancelled { payload_bytes: 0 })
        })
    }

    fn loaded_latency<'a>(
        &'a self,
        _: cfbench::plan::Direction,
        _: &'a CancellationToken,
    ) -> MeasurementFuture<'a> {
        unused_measurement()
    }

    fn download<'a>(
        &'a self,
        _: u64,
        _: Option<&'a str>,
        _: TransferTelemetry,
        _: &'a CancellationToken,
    ) -> MeasurementFuture<'a> {
        unused_measurement()
    }

    fn upload<'a>(
        &'a self,
        _: u64,
        _: TransferTelemetry,
        _: &'a CancellationToken,
    ) -> MeasurementFuture<'a> {
        unused_measurement()
    }
}

#[derive(Default)]
struct ChannelFillLifecycle {
    completed: Mutex<u32>,
    completed_notification: Condvar,
}

struct FailAfterAllRequestsWriter(Arc<ChannelFillLifecycle>);

impl Write for FailAfterAllRequestsWriter {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        let mut completed = self.0.completed.lock().unwrap();
        while *completed < 300 {
            completed = self.0.completed_notification.wait(completed).unwrap();
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "scripted failure after progress channel fills",
        ))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct CountingLatencyTransport {
    lifecycle: Arc<ChannelFillLifecycle>,
    expected: u32,
}

impl MeasurementTransport for CountingLatencyTransport {
    fn latency<'a>(&'a self, _: &'a CancellationToken) -> MeasurementFuture<'a> {
        let lifecycle = self.lifecycle.clone();
        let expected = self.expected;
        Box::pin(async move {
            let mut completed = lifecycle.completed.lock().unwrap();
            *completed += 1;
            if *completed == expected {
                lifecycle.completed_notification.notify_one();
            }
            drop(completed);
            Ok(TimingObservation::from_millis(20.0, 20.0, 10.0, 0, "1.1"))
        })
    }

    fn loaded_latency<'a>(
        &'a self,
        _: cfbench::plan::Direction,
        _: &'a CancellationToken,
    ) -> MeasurementFuture<'a> {
        unused_measurement()
    }

    fn download<'a>(
        &'a self,
        _: u64,
        _: Option<&'a str>,
        _: TransferTelemetry,
        _: &'a CancellationToken,
    ) -> MeasurementFuture<'a> {
        unused_measurement()
    }

    fn upload<'a>(
        &'a self,
        _: u64,
        _: TransferTelemetry,
        _: &'a CancellationToken,
    ) -> MeasurementFuture<'a> {
        unused_measurement()
    }
}

struct InstallSignal(Arc<AtomicBool>);

impl Future for InstallSignal {
    type Output = std::io::Result<()>;

    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        self.0.store(true, Ordering::SeqCst);
        Poll::Pending
    }
}

struct InstallAwareTransport(Arc<AtomicBool>);

impl MeasurementTransport for InstallAwareTransport {
    fn latency<'a>(&'a self, _: &'a CancellationToken) -> MeasurementFuture<'a> {
        assert!(self.0.load(Ordering::SeqCst));
        Box::pin(async { Ok(TimingObservation::from_millis(20.0, 20.0, 10.0, 0, "1.1")) })
    }

    fn loaded_latency<'a>(
        &'a self,
        _: cfbench::plan::Direction,
        _: &'a CancellationToken,
    ) -> MeasurementFuture<'a> {
        unused_measurement()
    }

    fn download<'a>(
        &'a self,
        _: u64,
        _: Option<&'a str>,
        _: TransferTelemetry,
        _: &'a CancellationToken,
    ) -> MeasurementFuture<'a> {
        unused_measurement()
    }

    fn upload<'a>(
        &'a self,
        _: u64,
        _: TransferTelemetry,
        _: &'a CancellationToken,
    ) -> MeasurementFuture<'a> {
        unused_measurement()
    }
}

fn unused_measurement<'a>() -> MeasurementFuture<'a> {
    Box::pin(async { Err(TransportError::Cancelled { payload_bytes: 0 }) })
}

struct ReadyWithFinalOperation(Arc<AtomicBool>);

impl Future for ReadyWithFinalOperation {
    type Output = std::io::Result<()>;

    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        if self.0.load(Ordering::SeqCst) {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }
}

struct ConcurrentSuccessTransport(Arc<AtomicBool>);

impl MeasurementTransport for ConcurrentSuccessTransport {
    fn latency<'a>(&'a self, _: &'a CancellationToken) -> MeasurementFuture<'a> {
        Box::pin(PendingThenSuccess {
            first_poll: true,
            final_operation_polled: self.0.clone(),
        })
    }

    fn loaded_latency<'a>(
        &'a self,
        _: cfbench::plan::Direction,
        _: &'a CancellationToken,
    ) -> MeasurementFuture<'a> {
        unused_measurement()
    }

    fn download<'a>(
        &'a self,
        _: u64,
        _: Option<&'a str>,
        _: TransferTelemetry,
        _: &'a CancellationToken,
    ) -> MeasurementFuture<'a> {
        unused_measurement()
    }

    fn upload<'a>(
        &'a self,
        _: u64,
        _: TransferTelemetry,
        _: &'a CancellationToken,
    ) -> MeasurementFuture<'a> {
        unused_measurement()
    }
}

struct ConcurrentErrorTransport(Arc<AtomicBool>);

impl MeasurementTransport for ConcurrentErrorTransport {
    fn latency<'a>(&'a self, _: &'a CancellationToken) -> MeasurementFuture<'a> {
        Box::pin(PendingThenError {
            first_poll: true,
            final_operation_polled: self.0.clone(),
        })
    }

    fn loaded_latency<'a>(
        &'a self,
        _: cfbench::plan::Direction,
        _: &'a CancellationToken,
    ) -> MeasurementFuture<'a> {
        unused_measurement()
    }

    fn download<'a>(
        &'a self,
        _: u64,
        _: Option<&'a str>,
        _: TransferTelemetry,
        _: &'a CancellationToken,
    ) -> MeasurementFuture<'a> {
        unused_measurement()
    }

    fn upload<'a>(
        &'a self,
        _: u64,
        _: TransferTelemetry,
        _: &'a CancellationToken,
    ) -> MeasurementFuture<'a> {
        unused_measurement()
    }
}

struct PendingThenSuccess {
    first_poll: bool,
    final_operation_polled: Arc<AtomicBool>,
}

struct PendingThenError {
    first_poll: bool,
    final_operation_polled: Arc<AtomicBool>,
}

impl Future for PendingThenError {
    type Output = Result<TimingObservation, TransportError>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        if self.first_poll {
            self.first_poll = false;
            self.final_operation_polled.store(true, Ordering::SeqCst);
            context.waker().wake_by_ref();
            Poll::Pending
        } else {
            Poll::Ready(Err(TransportError::HttpStatus {
                endpoint: "https://fixture.invalid/__down".to_owned(),
                status: 503,
                payload_bytes: 0,
            }))
        }
    }
}

impl Future for PendingThenSuccess {
    type Output = Result<TimingObservation, TransportError>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        if self.first_poll {
            self.first_poll = false;
            self.final_operation_polled.store(true, Ordering::SeqCst);
            context.waker().wake_by_ref();
            Poll::Pending
        } else {
            Poll::Ready(Ok(TimingObservation::from_millis(
                20.0, 20.0, 10.0, 0, "1.1",
            )))
        }
    }
}

struct ErrorTransport(String);

impl MeasurementTransport for ErrorTransport {
    fn latency<'a>(&'a self, _: &'a CancellationToken) -> MeasurementFuture<'a> {
        let endpoint = self.0.clone();
        Box::pin(async move {
            Err(TransportError::HttpStatus {
                endpoint,
                status: 503,
                payload_bytes: 0,
            })
        })
    }

    fn loaded_latency<'a>(
        &'a self,
        _: cfbench::plan::Direction,
        _: &'a CancellationToken,
    ) -> MeasurementFuture<'a> {
        unused_measurement()
    }

    fn download<'a>(
        &'a self,
        _: u64,
        _: Option<&'a str>,
        _: TransferTelemetry,
        _: &'a CancellationToken,
    ) -> MeasurementFuture<'a> {
        unused_measurement()
    }

    fn upload<'a>(
        &'a self,
        _: u64,
        _: TransferTelemetry,
        _: &'a CancellationToken,
    ) -> MeasurementFuture<'a> {
        unused_measurement()
    }
}
