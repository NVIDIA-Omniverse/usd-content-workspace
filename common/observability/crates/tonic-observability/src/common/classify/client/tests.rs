// SPDX-FileCopyrightText: Copyright (c) 2024-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use super::{find_code_in_source_chain, h2_err_to_code};
use h2::Reason;
use std::convert::Infallible;
use std::error::Error as StdError;
use std::fmt;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;
use tonic::{Code, Status};

#[test]
fn h2_errors() {
    let mapping = [
        Reason::NO_ERROR,
        Reason::PROTOCOL_ERROR,
        Reason::INTERNAL_ERROR,
        Reason::FLOW_CONTROL_ERROR,
        Reason::SETTINGS_TIMEOUT,
        Reason::STREAM_CLOSED,
        Reason::FRAME_SIZE_ERROR,
        Reason::REFUSED_STREAM,
        Reason::CANCEL,
        Reason::COMPRESSION_ERROR,
        Reason::CONNECT_ERROR,
        Reason::ENHANCE_YOUR_CALM,
        Reason::INADEQUATE_SECURITY,
        Reason::HTTP_1_1_REQUIRED,
        Reason::from(14),
    ];
    for (index, reason) in mapping.into_iter().enumerate() {
        assert_eq!(reason, Reason::from(index as u32));
        let err = h2::Error::from(reason);
        let actual = h2_err_to_code(&err);
        let expected = Status::try_from_error(err.into()).unwrap().code();
        assert_eq!(expected, actual, "{reason:?} ({index})");
    }
}

#[test]
fn downcast_err_success() {
    // this test is a pain to write:
    // * we cannot even construct a `tonic::transport::Error`.
    // * we cannot directly construct a `hyper::Error`, so a ridiculous
    //   workaround is needed for that.
    let hef = HyperErrorFactory::new();
    let errors: Vec<BoxError> = vec![
        Nested::depth1(Status::aborted("foo")),
        Nested::depth1(tonic::TimeoutExpired(())),
        Nested::depth1(tonic::ConnectError(TestError.into())),
        Nested::depth1(hef.timeout()),
        Nested::depth1(hef.canceled()),
        Nested::depth1(hef.h2(Reason::ENHANCE_YOUR_CALM)),
        Nested::depth2(Status::aborted("foo")),
        Nested::depth2(tonic::TimeoutExpired(())),
        Nested::depth3(Status::aborted("foo")),
        Nested::depth3(tonic::TimeoutExpired(())),
    ];
    for err in errors {
        let code = find_code_in_source_chain(Some(&*err));
        let status = Status::try_from_error(err).unwrap();
        assert_eq!(status.code(), code);
    }
}

#[test]
fn downcast_err_fallthrough() {
    let hef = HyperErrorFactory::new();

    let code = find_code_in_source_chain(None);
    assert_eq!(Code::Unknown, code);

    let errors: Vec<BoxError> = vec![
        Nested::depth1(hef.other()),
        Nested::depth1(TestError),
        // remember, the actual error chain is `tonic::transport::Error`!
        // so even though the case below is basically
        // `Status::try_from_error(h2::Error)` and succeeds succeeds, outside of
        // the test it would be
        // `Status::try_from_error(tonic::transport::Error(h2::Error))`, and
        // would fail.
        // while we could wrap the errors in a fake `tonic::transport::Error`,
        // that's basically what `Nested` does.
        // ```
        // Nested::depth0(h2::Error::from(Reason::ENHANCE_YOUR_CALM)),
        // ```
        Nested::depth2(TestError),
        Nested::depth2(h2::Error::from(Reason::ENHANCE_YOUR_CALM)),
        Nested::depth3(TestError),
        Nested::depth3(h2::Error::from(Reason::ENHANCE_YOUR_CALM)),
    ];
    for err in errors {
        let code = find_code_in_source_chain(Some(&*err));
        let _err = Status::try_from_error(err).unwrap_err();
        assert_eq!(Code::Unknown, code);
    }
}

type BoxError = Box<dyn StdError + Send + Sync + 'static>;

#[derive(Debug)]
struct TestError;

impl fmt::Display for TestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("test")
    }
}

impl StdError for TestError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        None
    }
}

#[derive(Debug)]
struct Nested(BoxError);

impl Nested {
    // remember, the actual error chain is `tonic::transport::Error`!
    fn depth1(e: impl StdError + Send + Sync + 'static) -> BoxError {
        Box::new(e)
    }

    fn depth2(e: impl StdError + Send + Sync + 'static) -> BoxError {
        Box::new(Self(Self::depth1(e)))
    }

    fn depth3(e: impl StdError + Send + Sync + 'static) -> BoxError {
        Box::new(Self(Self::depth2(e)))
    }
}

impl fmt::Display for Nested {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl StdError for Nested {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        Some(&*self.0)
    }
}

struct HyperErrorFactory {
    rt: tokio::runtime::Runtime,
}

impl HyperErrorFactory {
    fn new() -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        Self { rt }
    }

    /// Returns a `hyper::Error` where `is_canceled()` is true.
    fn canceled(&self) -> hyper::Error {
        let e = self.rt.block_on(hyper_error_canceled());
        assert!(e.is_canceled(), "hyper error construction sanity check");
        e
    }

    /// Returns a `hyper::Error` where `is_timeout()` is true.
    fn timeout(&self) -> hyper::Error {
        let e = self.rt.block_on(hyper_error_timeout());
        assert!(e.is_timeout(), "hyper error construction sanity check");
        e
    }

    /// Returns a `hyper::Error` with a `h2::Error` source.
    fn h2(&self, reason: Reason) -> hyper::Error {
        let e = self.rt.block_on(hyper_error_h2(reason));
        let actual = e
            .source()
            .and_then(|src| src.downcast_ref::<h2::Error>())
            .and_then(h2::Error::reason);
        assert_eq!(
            Some(reason),
            actual,
            "hyper error construction sanity check"
        );
        e
    }

    /// Returns a `hyper::Error` that is not a timeout, not canceled, and has
    /// no `h2::Error` source.
    fn other(&self) -> hyper::Error {
        let e = self.rt.block_on(hyper_error_other());
        assert!(
            !e.is_timeout(),
            "hyper error construction sanity check (timeout)"
        );
        assert!(
            !e.is_canceled(),
            "hyper error construction sanity check (canceled)"
        );
        assert!(
            e.source()
                .and_then(|src| src.downcast_ref::<h2::Error>())
                .is_none(),
            "hyper error construction sanity check (h2)"
        );
        e
    }
}

struct EmptyBody;

impl EmptyBody {
    fn request() -> hyper::Request<Self> {
        hyper::Request::builder()
            .uri("http://localhost/")
            .body(Self)
            .unwrap()
    }
}

impl http_body::Body for EmptyBody {
    type Data = bytes::Bytes;
    type Error = Infallible;

    fn poll_frame(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        Poll::Ready(None)
    }

    fn is_end_stream(&self) -> bool {
        true
    }
}

/// Returns a `hyper::Error` where `is_canceled()` is true.
///
/// This ridiculous workaround is needed because there's no public API to
/// actually construct a `hyper::Error`.
async fn hyper_error_canceled() -> hyper::Error {
    let (client_io, _server_io) = tokio::io::duplex(1);
    let client_io = hyper_util::rt::TokioIo::new(client_io);

    // just don't run a server, although this relies on HTTP/1

    // emulate a client
    let (mut send_request, conn) = hyper::client::conn::http1::handshake(client_io)
        .await
        .unwrap();

    // drop the client connection
    drop(conn);

    let req = EmptyBody::request();
    send_request.send_request(req).await.unwrap_err()
}

/// Returns a `hyper::Error` where `is_timeout()` is true.
///
/// This ridiculous workaround is needed because there's no public API to
/// actually construct a `hyper::Error`.
async fn hyper_error_timeout() -> hyper::Error {
    let (client_io, server_io) = tokio::io::duplex(65536);
    let exec = hyper_util::rt::TokioExecutor::new();
    let client_io = hyper_util::rt::TokioIo::new(client_io);
    let timer = hyper_util::rt::TokioTimer::default();

    // run the server in the background
    tokio::spawn(async move {
        // complete the handshake...
        let conn = h2::server::handshake(server_io).await.unwrap();
        // ... then stop driving the connection. client PING frames won't be
        // answered. however, the connection must be kept "alive":
        futures::future::pending::<()>().await;
        drop(conn);
    });

    // emulate a client with a short time-out
    let (mut send_request, conn) = hyper::client::conn::http2::Builder::new(exec)
        .timer(timer)
        .keep_alive_interval(Some(Duration::from_millis(10)))
        .keep_alive_timeout(Duration::from_millis(10))
        .keep_alive_while_idle(true)
        .handshake(client_io)
        .await
        .unwrap();

    // drive the client connection in the background
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let req = EmptyBody::request();
    send_request.send_request(req).await.unwrap_err()
}

/// Returns a `hyper::Error` with a `h2::Error` source.
///
/// This ridiculous workaround is needed because there's no public API to
/// actually construct a `hyper::Error`.
async fn hyper_error_h2(reason: Reason) -> hyper::Error {
    let (client_io, server_io) = tokio::io::duplex(65536);
    let exec = hyper_util::rt::TokioExecutor::new();
    let client_io = hyper_util::rt::TokioIo::new(client_io);

    // run the server in the background
    tokio::spawn(async move {
        // complete the handshake
        let mut conn = h2::server::handshake(server_io).await.unwrap();
        // accept one stream...
        if let Some(Ok((_request, mut send_response))) = conn.accept().await {
            // ...reset it
            send_response.send_reset(reason);
        }
        // keep driving the connection so the RST_STREAM frame is flushed
        let _ = futures::future::poll_fn(|cx| conn.poll_closed(cx)).await;
        drop(conn);
    });

    // emulate a client
    let (mut send_request, conn) = hyper::client::conn::http2::handshake(exec, client_io)
        .await
        .unwrap();

    // drive the client connection in the background
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let req = EmptyBody::request();
    send_request.send_request(req).await.unwrap_err()
}

/// Returns a `hyper::Error` that is not a timeout, not canceled, and has no
/// `h2::Error` source.
///
/// This ridiculous workaround is needed because there's no public API to
/// actually construct a `hyper::Error`.
async fn hyper_error_other() -> hyper::Error {
    let (client_io, mut server_io) = tokio::io::duplex(65536);
    let client_io = hyper_util::rt::TokioIo::new(client_io);

    // read the client's request, send back a partial HTTP response, then
    // close the connection. hyper sees an incomplete message.
    tokio::spawn(async move {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let mut buf = [0u8; 1024];
        let _ = server_io.read(&mut buf).await;
        server_io.write_all(b"HTTP/1.1 200 OK\r\n").await.unwrap();
        server_io.shutdown().await.unwrap();
    });

    let (mut send_request, conn) = hyper::client::conn::http1::handshake(client_io)
        .await
        .unwrap();

    tokio::spawn(async move {
        let _ = conn.await;
    });

    let req = EmptyBody::request();
    send_request.send_request(req).await.unwrap_err()
}
