// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
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

use std::net::SocketAddr;

use futures::future::BoxFuture;
use tokio::net::TcpSocket;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::transport::Server;

use super::error::Error;

/// Create a Kubernetes Health service that runs at the specified address.
pub fn create_probe_service(listen_addr: SocketAddr) -> Result<BoxFuture<'static, Result<(), Error>>, Error> {
    let socket = create_and_bind(listen_addr).map_err(|source| Error::new_with_source("bind failed", source))?;
    let listen_addr = socket
        .local_addr()
        .map_err(|source| Error::new_with_source("failed to get the local addr", source))?;
    let listener = TcpListenerStream::new(
        socket
            .listen(1)
            .map_err(|source| Error::new_with_source("listen failed", source))?,
    );

    let (_reporter, service) = tonic_health::server::health_reporter();
    let builder = Server::builder().add_service(service);

    Ok(Box::pin(async move {
        tracing::info!("grpc probe service is listening on {listen_addr}");

        builder
            .serve_with_incoming(listener)
            .await
            .map_err(|source| Error::new_with_source("serve failed", source))
    }))
}

fn create_and_bind(addr: SocketAddr) -> std::io::Result<TcpSocket> {
    let socket = TcpSocket::new_v4()?;
    socket.set_reuseaddr(true)?;
    socket.set_reuseport(true)?;
    socket.set_nodelay(true)?;
    socket.set_keepalive(true)?;
    socket.bind(addr)?;

    Ok(socket)
}
