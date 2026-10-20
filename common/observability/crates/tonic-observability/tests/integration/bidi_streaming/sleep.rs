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

use crate::harness::prelude::*;
use pin_project_lite::pin_project;
use protos::bidi_streaming_client::BidiStreamingClient as Client;
use protos::bidi_streaming_server::{BidiStreaming as Service, BidiStreamingServer as Server};
use std::task::Poll;
use std::time::Duration;

struct Svc;

#[async_trait]
impl Service for Svc {
    type EchoStream = SleepStream<Result<EchoResponse>>;

    #[instrument(name = "echo (server)", level = "info", skip_all)]
    async fn echo(
        &self,
        request: Request<Streaming<EchoRequest>>,
    ) -> Result<Response<Self::EchoStream>> {
        let mut stream = request.into_inner();
        while let Some(_message) = stream.next().await {}

        tokio::time::sleep(Duration::from_millis(30)).await;

        let item = Ok(echo_resp("Echo"));
        let stream = SleepStream::new(Duration::from_millis(30), item);
        Ok(Response::new(stream))
    }
}

/// This test ensures we hit all the Poll::Pending states in the layers...
#[test]
fn test() {
    let (request, _req_size) = make_req(echo_req("Echo"));

    let message = vec![Ok(echo_resp("Echo"))];
    let (response, _resp_size) = make_resp(message);

    let resource = resource();
    let svc = Server::new(Svc);

    let output = harness(&resource, svc, async |make| {
        let mut client: Client<_> = make.default();

        // the sleep stream needs a tokio reactor running
        let req = SleepStream::new(Duration::from_millis(10), request.into_inner());

        client
            .echo(req)
            .instrument(info_span!("echo (client)"))
            .await
            .consume()
            .await
    });

    assert_eq!(output.response(), &response);

    let actual_spans = output.spans(&resource);

    actual_spans.assert(expected_hierarchy! {
        "test" => {
            "echo (client)" => {
                "example.BidiStreaming/Echo (client)" => {
                    "example.BidiStreaming/Echo (server)" => {
                        "echo (server)"
                    }
                }
            },
            "streaming (client)"
        },
    });
    actual_spans.hierarchy.assert_server_streaming();

    // the exact spans and metrics aren't interesting for this test
}

pin_project! {
    #[derive(Debug)]
    struct SleepStream<T> {
        #[pin]
        sleep: tokio::time::Sleep,
        item: Option<T>,
    }
}

impl<T> SleepStream<T> {
    fn new(duration: Duration, item: T) -> Self {
        Self {
            sleep: tokio::time::sleep(duration),
            item: Some(item),
        }
    }
}

impl<T: std::fmt::Debug + Send + Sync + 'static> futures::Stream for SleepStream<T> {
    type Item = T;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let this = self.project();
        match this.sleep.poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(()) => Poll::Ready(this.item.take()),
        }
    }
}
