use cfbench::output::{render_compact_progress, render_progress};
use cfbench::plan::Direction;
use cfbench::progress::{ProgressEvent, ProgressFailureKind, ProgressReporter, ProgressStage};

#[test]
fn formats_individual_transfer_progress_without_terminal_control() {
    let line = render_progress(&ProgressEvent::TransferCompleted {
        direction: Direction::Download,
        requested_bytes: 100_000_000,
        current: 1,
        total: 3,
        bps: 676_870_000,
        adjusted_duration_ms: 1_188.4,
    });

    assert_eq!(line, "[download 100 MB 1/3] 676.87 Mbps — 1.19 s");
    assert!(!line.contains(['\r', '\u{1b}']));
}

#[test]
fn compact_progress_formats_primary_request_lifecycle() {
    let cases = [
        (
            ProgressEvent::RequestStarted {
                stage: ProgressStage::Latency,
                current: Some(2),
                total: Some(20),
            },
            Some("Latency 2/20"),
        ),
        (
            ProgressEvent::LatencyCompleted {
                current: 2,
                total: 20,
                latency_ms: 12.4,
            },
            Some("Latency 2/20 - 12.40 ms"),
        ),
        (
            ProgressEvent::RequestStarted {
                stage: ProgressStage::Transfer {
                    direction: Direction::Download,
                    requested_bytes: 100_000_000,
                },
                current: Some(2),
                total: Some(3),
            },
            Some("Download 100 MB 2/3"),
        ),
        (
            ProgressEvent::TransferCompleted {
                direction: Direction::Upload,
                requested_bytes: 50_000_000,
                current: 1,
                total: 3,
                bps: 216_900_000,
                adjusted_duration_ms: 1_200.0,
            },
            Some("Upload 50 MB 1/3 - 216.90 Mbps"),
        ),
    ];

    for (event, expected) in cases {
        let actual = render_compact_progress(&event);
        assert_eq!(actual.as_deref(), expected);
        assert!(!actual.unwrap().contains(['\r', '\u{1b}']));
    }
}

#[test]
fn compact_progress_keeps_loaded_latency_events_out_of_active_display() {
    let events = [
        ProgressEvent::LoadedLatencyCompleted {
            direction: Direction::Download,
            sequence: 4,
            latency_ms: 12.4,
        },
        ProgressEvent::RequestFailed {
            stage: ProgressStage::LoadedLatency {
                direction: Direction::Download,
            },
            current: None,
            total: None,
            kind: ProgressFailureKind::Timeout,
        },
    ];

    for event in events {
        assert_eq!(render_compact_progress(&event), None);
    }
}

#[test]
fn compact_progress_formats_only_safe_request_failure_categories() {
    let rendered = render_compact_progress(&ProgressEvent::RequestFailed {
        stage: ProgressStage::Transfer {
            direction: Direction::Upload,
            requested_bytes: 50_000_000,
        },
        current: Some(1),
        total: Some(3),
        kind: ProgressFailureKind::Timeout,
    })
    .expect("request failures retain the safe failure category");

    assert_eq!(rendered, "Upload 50 MB 1/3 - failed: timeout");
    assert!(!rendered.contains("fixture.invalid"));
    assert!(!rendered.contains("https://"));
}

#[test]
fn formats_all_progress_event_lines_with_documented_units_and_punctuation() {
    let cases = [
        (
            ProgressEvent::LatencyCompleted {
                current: 1,
                total: 20,
                latency_ms: 22.8,
            },
            "[latency 1/20] 22.80 ms",
        ),
        (
            ProgressEvent::TransferCompleted {
                direction: Direction::Download,
                requested_bytes: 100_000,
                current: 1,
                total: 9,
                bps: 91_420_000,
                adjusted_duration_ms: 11.0,
            },
            "[download 100 KB 1/9] 91.42 Mbps — 11.0 ms",
        ),
        (
            ProgressEvent::LoadedLatencyCompleted {
                direction: Direction::Download,
                sequence: 1,
                latency_ms: 25.4,
            },
            "[loaded/download 1] 25.40 ms",
        ),
        (
            ProgressEvent::RequestFailed {
                stage: ProgressStage::Transfer {
                    direction: Direction::Download,
                    requested_bytes: 100_000_000,
                },
                current: Some(1),
                total: Some(3),
                kind: ProgressFailureKind::HttpStatus(403),
            },
            "[download 100 MB 1/3] failed — HTTP 403",
        ),
        (
            ProgressEvent::DirectionFinished {
                direction: Direction::Download,
            },
            "[download] larger payload groups skipped — request duration threshold reached",
        ),
    ];

    for (event, expected) in cases {
        let line = render_progress(&event);
        assert_eq!(line, expected);
        assert!(!line.contains(['\r', '\u{1b}']));
    }
}

#[test]
fn formatter_uses_unavailable_for_non_finite_measurements() {
    let latency = render_progress(&ProgressEvent::LatencyCompleted {
        current: 1,
        total: 1,
        latency_ms: f64::NAN,
    });
    let transfer = render_progress(&ProgressEvent::TransferCompleted {
        direction: Direction::Upload,
        requested_bytes: 1_000_000,
        current: 1,
        total: 6,
        bps: 328_090_000,
        adjusted_duration_ms: f64::INFINITY,
    });
    let loaded = render_progress(&ProgressEvent::LoadedLatencyCompleted {
        direction: Direction::Upload,
        sequence: 1,
        latency_ms: f64::NEG_INFINITY,
    });

    assert_eq!(latency, "[latency 1/1] unavailable");
    assert_eq!(transfer, "[upload 1 MB 1/6] unavailable");
    assert_eq!(loaded, "[loaded/upload 1] unavailable");
    assert!(!format!("{latency}\n{transfer}\n{loaded}").contains("NaN"));
    assert!(!format!("{latency}\n{transfer}\n{loaded}").contains("inf"));
}

#[test]
fn formats_safe_failure_categories_without_transport_details() {
    let cases = [
        (ProgressFailureKind::Timeout, "timeout"),
        (ProgressFailureKind::Cancelled, "cancelled"),
        (ProgressFailureKind::BodyStream, "body stream"),
        (ProgressFailureKind::PayloadMismatch, "payload mismatch"),
        (
            ProgressFailureKind::InvalidMeasurement,
            "invalid measurement",
        ),
        (ProgressFailureKind::Request, "request failed"),
    ];

    for (kind, expected_detail) in cases {
        let line = render_progress(&ProgressEvent::RequestFailed {
            stage: ProgressStage::LoadedLatency {
                direction: Direction::Upload,
            },
            current: None,
            total: None,
            kind,
        });
        assert_eq!(line, format!("[loaded/upload] failed — {expected_detail}"));
    }
}

#[test]
fn full_or_closed_progress_channel_never_blocks_or_fails() {
    let (reporter, receiver) = ProgressReporter::channel(1);

    let event = ProgressEvent::DirectionFinished {
        direction: Direction::Download,
    };
    reporter.emit(event.clone());
    reporter.emit(event.clone());
    assert!(matches!(
        receiver.try_recv(),
        Ok(ProgressEvent::DirectionFinished { .. })
    ));

    drop(receiver);
    reporter.emit(event.clone());
    ProgressReporter::disabled().emit(event);
}
