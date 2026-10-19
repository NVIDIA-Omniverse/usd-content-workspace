// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use axum::extract::Request;
use bytes::{Bytes, BytesMut};
use futures::{pin_mut, Stream, StreamExt};
use snafu::{IntoError as _, Location, Snafu};
use storage_core_types::error::{StorageError, StorageErrorExt};

use crate::traits::BoxFuture;

/// Create a data future that reads exactly `expected_len` bytes from the request body.
/// Trailing request bytes, if any, will be discarded.
pub fn collect_request_body_future(request: Request, expected_len: usize) -> BoxFuture<Bytes> {
    collect_body_streamed_future(request.into_body().into_data_stream(), Some, expected_len)
}

/// Create a data future that reads exactly `expected_len` bytes from the consecutive
/// Chunk messages in the stream.
/// Trailing request bytes, if any, will be discarded.
pub fn collect_body_streamed_future<T, E>(
    stream: impl Stream<Item = Result<T, E>> + Send + 'static,
    try_into_chunk: impl Fn(T) -> Option<Bytes> + Send + 'static,
    expected_len: usize,
) -> BoxFuture<Bytes>
where
    E: std::error::Error + Send + Sync + 'static,
{
    Box::pin(async move {
        collect_body_streamed(stream, try_into_chunk, expected_len)
            .await
            .map_err(|e| Box::new(e) as StorageError)
    })
}

async fn collect_body_streamed<T, E>(
    message_stream: impl Stream<Item = Result<T, E>> + Send,
    try_into_chunk: impl Fn(T) -> Option<Bytes> + Send,
    expected_len: usize,
) -> Result<Bytes, CollectBodyError>
where
    E: std::error::Error + Send + Sync + 'static,
{
    let mut collected = BytesMut::new();

    pin_mut!(message_stream);

    let mut remaining_len = expected_len;
    while remaining_len != 0 {
        let message = message_stream
            .next()
            .await
            .ok_or_else(|| {
                collect_body_error::BodyTooShort {
                    actual_len: collected.len(),
                    expected_len,
                }
                .build()
            })?
            .map_err(|source| collect_body_error::Read.into_error(Box::new(source)))?;

        let chunk = try_into_chunk(message).ok_or_else(|| collect_body_error::ChunkMessageExpected.build())?;

        if collected.is_empty() {
            if chunk.len() == expected_len {
                return Ok(chunk);
            }

            collected.reserve(expected_len);
        }

        let chunk = chunk
            .split_at_checked(remaining_len)
            .map(|(prefix, _)| prefix)
            .unwrap_or(&chunk);
        collected.extend_from_slice(chunk);

        remaining_len -= chunk.len();
    }

    Ok(collected.freeze())
}

#[derive(Snafu, Debug)]
#[snafu(visibility(pub), module, context(suffix(false)))]
pub enum CollectBodyError {
    #[snafu(display("failed to collect body: {source:?}"))]
    Read {
        source: Box<dyn std::error::Error + Send + Sync>,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("request body is too short (expected {expected_len}, got {actual_len})"))]
    BodyTooShort {
        expected_len: usize,
        actual_len: usize,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("chunk message expected"))]
    ChunkMessageExpected {
        #[snafu(provide, implicit)]
        location: Location,
    },
}

impl StorageErrorExt for CollectBodyError {
    fn tonic_status(&self) -> tonic::Status {
        use tonic::Status;

        match self {
            Self::Read { .. } => Status::internal(self.to_string()),
            Self::BodyTooShort { .. } | Self::ChunkMessageExpected { .. } => Status::invalid_argument(self.to_string()),
        }
    }

    fn http_status(&self) -> http::StatusCode {
        use http::StatusCode;

        match self {
            Self::Read { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            Self::BodyTooShort { .. } | Self::ChunkMessageExpected { .. } => StatusCode::BAD_REQUEST,
        }
    }

    fn get_size(&self) -> usize {
        std::mem::size_of::<Self>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream;
    use std::io;

    fn byte_sequence(chunks: &[&'static [u8]]) -> Vec<Bytes> {
        chunks.iter().copied().map(Bytes::from_static).collect()
    }

    fn chunk_stream(chunks: &[&'static [u8]]) -> impl Stream<Item = Result<Bytes, io::Error>> + use<> {
        stream::iter(byte_sequence(chunks).into_iter().map(Ok))
    }

    #[tokio::test]
    async fn collect_body_streamed_reads_from_multiple_chunks() {
        let body = collect_body_streamed(chunk_stream(&[b"ab", b"cd", b"ef"]), Some, 6)
            .await
            .unwrap();

        assert_eq!(body, Bytes::from_static(b"abcdef"));
    }

    #[tokio::test]
    async fn collect_body_streamed_ignores_trailing_bytes() {
        let body = collect_body_streamed(chunk_stream(&[b"ab", b"cdef", b"ignored"]), Some, 3)
            .await
            .unwrap();

        assert_eq!(body, Bytes::from_static(b"abc"));
    }

    #[tokio::test]
    async fn collect_body_streamed_reads_from_single_chunk() {
        let body = collect_body_streamed(chunk_stream(&[b"abcdef"]), Some, 6)
            .await
            .unwrap();

        assert_eq!(body, Bytes::from_static(b"abcdef"));
    }

    #[tokio::test]
    async fn collect_body_streamed_reads_zero_sized_object() {
        let body = collect_body_streamed(chunk_stream(&[b"ignored"]), Some, 0)
            .await
            .unwrap();

        assert!(body.is_empty());
    }

    #[tokio::test]
    async fn collect_body_streamed_errors_on_unexpected_message() {
        let messages = stream::iter([Ok::<_, io::Error>(None)]);

        let error = collect_body_streamed(messages, |chunk: Option<Bytes>| chunk, 1)
            .await
            .unwrap_err();

        assert!(matches!(error, CollectBodyError::ChunkMessageExpected { .. }));
    }

    #[tokio::test]
    async fn collect_body_streamed_errors_on_unexpected_end_of_stream() {
        let error = collect_body_streamed(chunk_stream(&[b"ab"]), Some, 3)
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            CollectBodyError::BodyTooShort {
                expected_len: 3,
                actual_len: 2,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn collect_body_streamed_errors_on_read_error() {
        let messages = stream::iter([Err::<Bytes, _>(io::Error::other("read failed"))]);

        let error = collect_body_streamed(messages, Some, 1).await.unwrap_err();

        assert!(matches!(
            error,
            CollectBodyError::Read { source, .. } if source.to_string() == "read failed"
        ));
    }
}
