use std::convert::Infallible;
use std::pin::Pin;
use std::sync::LazyLock;

use bytes::Bytes;
use futures_util::{Stream, stream};

const UPLOAD_CHUNK_BYTES: usize = 64 * 1024;
static ZERO_CHUNK: LazyLock<Bytes> = LazyLock::new(|| Bytes::from(vec![0_u8; UPLOAD_CHUNK_BYTES]));

pub type UploadStream =
    Pin<Box<dyn Stream<Item = Result<Bytes, Infallible>> + Send + Sync + 'static>>;

/// Creates a bounded-memory stream that yields exactly `bytes` zero bytes.
pub fn stream_upload(bytes: u64) -> (UploadStream, u64) {
    let stream = stream::unfold(bytes, |remaining| async move {
        if remaining == 0 {
            return None;
        }

        let emitted = remaining.min(UPLOAD_CHUNK_BYTES as u64) as usize;
        let chunk = ZERO_CHUNK.slice(..emitted);
        Some((Ok(chunk), remaining - emitted as u64))
    });
    (Box::pin(stream), bytes)
}
