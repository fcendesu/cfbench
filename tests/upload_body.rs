use std::sync::atomic::Ordering;
use std::time::Duration;

use cfbench::plan::Direction;
use cfbench::progress::{ProgressEvent, ProgressReporter, TransferTelemetry};
use cfbench::transport::upload_body::{stream_upload, stream_upload_with_telemetry};
use futures_util::StreamExt;

#[tokio::test]
async fn upload_stream_emits_exact_length_without_payload_sized_buffer() {
    let (mut body, content_length, yielded) = stream_upload(150_000);
    assert_eq!(content_length, 150_000);

    let mut lengths = Vec::new();
    while let Some(chunk) = body.next().await {
        lengths.push(chunk.expect("infallible upload chunk").len());
    }

    assert_eq!(lengths, vec![65_536, 65_536, 18_928]);
    assert_eq!(yielded.load(std::sync::atomic::Ordering::Relaxed), 150_000);
}

#[tokio::test]
async fn zero_length_upload_stream_is_empty() {
    let (mut body, content_length, yielded) = stream_upload(0);
    assert_eq!(content_length, 0);
    assert!(body.next().await.is_none());
    assert_eq!(yielded.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn upload_stream_reports_bounded_live_telemetry() {
    let (reporter, receiver) = ProgressReporter::channel(8);
    let telemetry = TransferTelemetry::new(reporter, Direction::Upload, 150_000, 1, 1);
    let (mut body, content_length, yielded) =
        stream_upload_with_telemetry(150_000, Some(telemetry));

    assert_eq!(content_length, 150_000);

    let mut lengths = Vec::new();
    while let Some(chunk) = body.next().await {
        lengths.push(chunk.expect("infallible upload chunk").len());
        if lengths.len() < 3 {
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    assert_eq!(yielded.load(Ordering::Relaxed), 150_000);
    assert!(lengths.iter().all(|length| *length <= 64 * 1024));
    assert_eq!(lengths, vec![65_536, 65_536, 18_928]);

    let events = receiver.into_iter().collect::<Vec<_>>();
    assert!(events.iter().any(|event| matches!(
        event,
        ProgressEvent::TransferAdvanced {
            direction: Direction::Upload,
            transferred_bytes,
            requested_bytes: 150_000,
            ..
        } if *transferred_bytes > 0 && *transferred_bytes <= 150_000
    )));
}
