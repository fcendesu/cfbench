use cfbench::transport::upload_body::stream_upload;
use futures_util::StreamExt;

#[tokio::test]
async fn upload_stream_emits_exact_length_without_payload_sized_buffer() {
    let (mut body, content_length) = stream_upload(150_000);
    assert_eq!(content_length, 150_000);

    let mut lengths = Vec::new();
    while let Some(chunk) = body.next().await {
        lengths.push(chunk.expect("infallible upload chunk").len());
    }

    assert_eq!(lengths, vec![65_536, 65_536, 18_928]);
}

#[tokio::test]
async fn zero_length_upload_stream_is_empty() {
    let (mut body, content_length) = stream_upload(0);
    assert_eq!(content_length, 0);
    assert!(body.next().await.is_none());
}
