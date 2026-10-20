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

use std::{collections::HashSet, net::SocketAddr, num::NonZero, str::FromStr, time::Duration};

use snafu::IntoError;
use tokio::sync::mpsc::Sender;
use tonic::transport::{channel::Change, Channel, ClientTlsConfig, Endpoint, Uri};
use tracing::{debug, warn};

use crate::balancer::utils::{Configure, Resolve, TokioLookupHostResolver};

pub use crate::balancer::error::Error;

pub mod error;
pub mod utils;

const SCHEME_HTTPS: &str = "https";
const SCHEME_HTTP: &str = "http";

const DEFAULT_PORT_HTTP: u16 = 80;
const DEFAULT_PORT_HTTPS: u16 = 442;

const DEFAULT_CHANNEL_CAPACITY: usize = 64;
const DEFAULT_CONNECTIONS_PER_HOST: NonZero<usize> = NonZero::new(1).unwrap();
const DEFAULT_DNS_REFRESH_INTERVAL: Duration = Duration::from_secs(5);

/// Load-balancing GRPC client builder.
#[derive(Debug)]
pub struct Builder {
    origin: Uri,
    tls_config: Option<ClientTlsConfig>,
    scheme: &'static str,
    host: String,
    port: u16,
    connections_per_host: NonZero<usize>,
    dns_refresh_interval: Duration,
}

impl Builder {
    /// Create a builder.
    pub fn new(uri: Uri) -> Result<Self, Error> {
        let scheme = uri.scheme().ok_or_else(|| error::UnsupportedScheme.build())?;

        let (default_port, scheme, tls_config) = if scheme.as_str().eq_ignore_ascii_case(SCHEME_HTTP) {
            (DEFAULT_PORT_HTTP, SCHEME_HTTP, None)
        } else if scheme.as_str().eq_ignore_ascii_case(SCHEME_HTTPS) {
            let tls_config = ClientTlsConfig::new().with_native_roots();
            (DEFAULT_PORT_HTTPS, SCHEME_HTTPS, Some(tls_config))
        } else {
            return error::UnsupportedScheme.fail();
        };

        let host = uri.host().ok_or_else(|| error::HostMissing.build())?;
        let port = uri.port_u16().unwrap_or(default_port);

        let origin = if let Some(port) = uri.port() {
            format!("{}://{}:{}", scheme, host, port)
        } else {
            format!("{}://{}", scheme, host)
        };
        let origin =
            Uri::try_from(&origin).map_err(|source| error::UnexpectedOriginParse.into_error(Box::new(source)))?;

        Ok(Self {
            host: host.into(),
            connections_per_host: DEFAULT_CONNECTIONS_PER_HOST,
            dns_refresh_interval: DEFAULT_DNS_REFRESH_INTERVAL,
            port,
            scheme,
            origin,
            tls_config,
        })
    }

    /// Override the number of connections per a single resolved endpoint.
    pub fn connections_per_host(mut self, value: NonZero<usize>) -> Self {
        self.connections_per_host = value;
        self
    }

    /// Override the default refresh interval.
    pub fn dns_refresh_interval(mut self, value: Duration) -> Self {
        self.dns_refresh_interval = value;
        self
    }

    /// Create a channel.
    pub async fn connect<C>(self, configure: C) -> Result<Channel, Error>
    where
        C: Configure,
    {
        self.connect_with_resolve(configure, TokioLookupHostResolver).await
    }

    /// Create a channel using a custom DNS resolver.
    pub async fn connect_with_resolve<C, R>(self, configure: C, resolve: R) -> Result<Channel, Error>
    where
        C: Configure,
        R: Resolve,
    {
        let (channel, sender) = Channel::balance_channel(DEFAULT_CHANNEL_CAPACITY);
        let context = RefreshContext {
            origin: self.origin,
            tls_config: self.tls_config,
            scheme: self.scheme,
            host: self.host,
            port: self.port,
            connections_per_host: self.connections_per_host,
            dns_refresh_interval: self.dns_refresh_interval,
            current_set: Default::default(),
            resolve,
            sender,
            configure,
        };

        context.connect().await?;

        Ok(channel)
    }
}

struct RefreshContext<R: Resolve, C: Configure> {
    origin: Uri,
    tls_config: Option<ClientTlsConfig>,
    scheme: &'static str,
    host: String,
    port: u16,
    connections_per_host: NonZero<usize>,
    dns_refresh_interval: Duration,
    sender: Sender<Change<EndpointKey, Endpoint>>,
    current_set: HashSet<SocketAddr>,
    resolve: R,
    configure: C,
}

impl<R: Resolve, C: Configure> RefreshContext<R, C> {
    /// Establish the initial connection pool and spawn a connection pool refresh subroutine.
    async fn connect(mut self) -> Result<(), Error> {
        enum State {
            EmptySet,
            AtLeastOne,
            AllError(Error),
        }

        debug!(host = ?self.host, port = ?self.port, "resolving a hostname");
        let addresses = self
            .resolve
            .resolve(&self.host, self.port)
            .await
            .map_err(|source| error::Resolve.into_error(Box::new(source)))?;

        let mut state = State::EmptySet;

        debug!(host = ?self.host, ?addresses, "establishing connections to the hosts");
        for addr in addresses {
            if let Err(error) = self.establish_for_host(addr).await {
                if matches!(state, State::EmptySet) {
                    state = State::AllError(error);
                }
            } else {
                state = State::AtLeastOne;
            }
        }

        match state {
            State::AllError(error) => {
                return Err(error);
            },
            State::EmptySet => {
                return error::ResolveEmptySet.fail();
            },
            State::AtLeastOne => {
                debug!(host = ?self.host, "at least one connection has been established");
            },
        };

        tokio::spawn(async move {
            loop {
                tokio::time::sleep(self.dns_refresh_interval).await;
                if !self.renew().await {
                    return;
                }
            }
        });

        Ok(())
    }

    /// Establish as many connections to the host as possible, but no less than one.
    /// Returns false if the associated channel has been closed.
    async fn establish_for_host(&mut self, addr: SocketAddr) -> Result<bool, Error> {
        debug!(?addr, "populating connections for a host");

        let uri = Uri::from_str(&format!("{}://{}", self.scheme, addr))
            .map_err(|source| error::UnexpectedEndpointUriParse.into_error(Box::new(source)))?;

        let endpoint = self.create_endpoint(uri).await?;
        for connection_id in 0..self.connections_per_host.get() {
            let maybe_continue = self
                .sender
                .send(Change::Insert(EndpointKey { addr, connection_id }, endpoint.clone()))
                .await
                .is_ok();
            if !maybe_continue {
                debug!("grpc channel has been closed");
                return Ok(false);
            }
        }

        self.current_set.insert(addr);

        debug!(?addr, "one or more connections have been established");

        Ok(true)
    }

    /// Create an endpoint builder for a URI.
    async fn create_endpoint(&mut self, uri: Uri) -> Result<Endpoint, Error> {
        let endpoint = Endpoint::from(uri).origin(self.origin.clone());
        let endpoint = if let Some(tls_config) = self.tls_config.as_ref() {
            endpoint
                .tls_config(tls_config.clone())
                .map_err(|source| error::InvalidTlsConfig.into_error(source))?
        } else {
            endpoint
        };

        let endpoint = self
            .configure
            .configure(endpoint)
            .map_err(|source| error::Configure.into_error(Box::new(source)))?;

        Ok(endpoint)
    }

    /// Renew the connection once.
    /// Returns false if the associated channel has been closed;
    async fn renew(&mut self) -> bool {
        debug!(host = ?self.host, "resolving a hostname");
        let Ok(addresses) = self
            .resolve
            .resolve(&self.host, self.port)
            .await
            .inspect_err(|error| warn!(host = ?self.host, ?error, "failed to resolve addresses for a hostname"))
        else {
            return true;
        };

        debug!(host = ?self.host, ?addresses, "hostname addresses resolved");

        let addresses_to_add: Vec<_> = addresses.symmetric_difference(&self.current_set).copied().collect();
        for addr in addresses_to_add {
            let Ok(do_continue) = self
                .establish_for_host(addr)
                .await
                .inspect_err(|error| warn!(?addr, ?error, "failed to establish connections to a host"))
            else {
                continue;
            };

            if !do_continue {
                return false;
            }
        }

        let addresses_to_remove: Vec<_> = self.current_set.symmetric_difference(&addresses).copied().collect();
        for addr in addresses_to_remove {
            self.current_set.remove(&addr);
            for connection_id in 0..self.connections_per_host.get() {
                if self
                    .sender
                    .send(Change::Remove(EndpointKey { addr, connection_id }))
                    .await
                    .is_err()
                {
                    debug!("grpc channel has been closed");
                    return false;
                }
            }
        }

        true
    }
}

#[derive(Eq, PartialEq, Hash, Clone, Copy, Debug)]
struct EndpointKey {
    addr: SocketAddr,
    connection_id: usize,
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{HashSet, VecDeque},
        net::SocketAddr,
        num::NonZero,
        time::Duration,
    };

    use assert_matches::assert_matches;
    use futures::stream::BoxStream;
    use snafu::Snafu;
    use tokio::{
        net::TcpSocket,
        sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
        time::{sleep, timeout},
    };
    use tokio_stream::wrappers::TcpListenerStream;
    use tokio_util::sync::{CancellationToken, DropGuard};
    use tonic::{
        transport::{Channel, Endpoint, Server, Uri},
        Request, Response, Status,
    };
    use tonic_health::pb::{
        health_client::HealthClient,
        health_server::{Health, HealthServer},
        HealthCheckRequest, HealthCheckResponse,
    };

    use crate::balancer::{
        error::Error,
        utils::{Configure, Resolve},
        Builder,
    };

    #[test_log::test(tokio::test)]
    async fn test_connect_with_resolve_one_host() {
        let (service_addr, mut service_rx, _service_guard) = create_health_service_mock().await;
        let resolver = ResolverMock {
            expected_host: "localhost".into(),
            expected_port: 1234,
            queue: [(vec![service_addr], usize::MAX)].into(),
        };

        // Make a request. The backend should receive the request.
        create_client(resolver)
            .await
            .check(HealthCheckRequest::default())
            .await
            .expect("check is expected to succeed");

        timeout(Duration::from_secs(5), service_rx.recv())
            .await
            .expect("notification wasn't received in time")
            .expect("channel has unexpectedly closed");
    }

    #[test_log::test(tokio::test)]
    async fn test_connect_with_resolve_many_hosts() {
        let (service_addr_1, mut service_rx_1, _service_guard_1) = create_health_service_mock().await;
        let (service_addr_2, mut service_rx_2, _service_guard_2) = create_health_service_mock().await;
        let resolver = ResolverMock {
            expected_host: "localhost".into(),
            expected_port: 1234,
            queue: [(vec![service_addr_1, service_addr_2], usize::MAX)].into(),
        };

        // Make some requests. Each backend should receive at least one request.
        let mut client = create_client(resolver).await;
        for _ in 0..32 {
            client
                .check(HealthCheckRequest::default())
                .await
                .expect("check is expected to succeed");
        }

        timeout(Duration::from_secs(5), service_rx_1.recv())
            .await
            .expect("notification wasn't received in time")
            .expect("channel has unexpectedly closed");
        timeout(Duration::from_secs(5), service_rx_2.recv())
            .await
            .expect("notification wasn't received in time")
            .expect("channel has unexpectedly closed");
    }

    #[test]
    fn test_builder_new_fails_for_unsupported_scheme() {
        let uri = Uri::from_static("ftp://myhost.mydomain");
        Builder::new(uri).expect_err("new is expected to fail");
    }

    #[test_log::test(tokio::test)]
    async fn test_builder_guesses_port_for_uri_without_port() {
        let (service_addr, mut service_rx, _service_guard) = create_health_service_mock().await;
        let resolver = ResolverMock {
            expected_host: "localhost".into(),
            expected_port: 80,
            queue: [(vec![service_addr], usize::MAX)].into(),
        };

        // Make a request. The backend should receive the request.
        create_client_with_address("http://localhost:80", resolver)
            .await
            .check(HealthCheckRequest::default())
            .await
            .expect("check is expected to succeed");

        timeout(Duration::from_secs(5), service_rx.recv())
            .await
            .expect("notification wasn't received in time")
            .expect("channel has unexpectedly closed");
    }

    #[test_log::test(tokio::test)]
    async fn test_connect_with_resolve_succeeds_when_at_least_one_host_is_reachable() {
        let (service_addr, mut service_rx, _service_guard) = create_health_service_mock().await;
        let resolver = ResolverMock {
            expected_host: "localhost".into(),
            expected_port: 1234,
            queue: [(vec![service_addr], usize::MAX)].into(),
        };

        // Make a request. The backend should receive it.
        create_client(resolver)
            .await
            .check(HealthCheckRequest::default())
            .await
            .expect("check is expected to succeed");

        timeout(Duration::from_secs(5), service_rx.recv())
            .await
            .expect("notification wasn't received in time")
            .expect("channel has unexpectedly closed");
    }

    #[test_log::test(tokio::test)]
    async fn test_connect_with_resolve_fails_when_resolve_returns_empty_set() {
        let resolver = ResolverMock {
            expected_host: "localhost".into(),
            expected_port: 1234,
            queue: [(vec![], usize::MAX)].into(),
        };

        // Attempt to create a connection. The routine must fail.
        let error = Builder::new(Uri::from_static("http://localhost:1234"))
            .expect("builder constructor failed")
            .connect_with_resolve(Identity, resolver)
            .await
            .expect_err("resolve is expected to fail");

        assert_matches!(error, Error::ResolveEmptySet { .. });
    }

    #[test_log::test(tokio::test)]
    async fn test_connect_with_resolve_renews_connection() {
        let (service_addr_1, mut service_rx_1, _service_guard_1) = create_health_service_mock().await;
        let (service_addr_2, mut service_rx_2, _service_guard_2) = create_health_service_mock().await;
        let resolver = ResolverMock {
            expected_host: "localhost".into(),
            expected_port: 1234,
            queue: [(vec![service_addr_1], 1), (vec![service_addr_2], usize::MAX)].into(),
        };

        let mut client = create_client(resolver).await;

        // Make a request. The first request must be routed to the first service.
        client
            .check(HealthCheckRequest::default())
            .await
            .expect("check is expected to succeed");

        timeout(Duration::from_secs(2), service_rx_1.recv())
            .await
            .expect("notification wasn't received in time")
            .expect("channel has unexpectedly closed");

        // Sleep until the connection has been refreshed. Connections to the first service should
        // have been dropped.
        sleep(Duration::from_secs(2)).await;

        // Make several requests and ensure they're all routed to the second service.
        for _ in 0..16 {
            client
                .check(HealthCheckRequest::default())
                .await
                .expect("check is expected to succeed");

            timeout(Duration::from_secs(2), service_rx_2.recv())
                .await
                .expect("notification wasn't received in time")
                .expect("channel has unexpectedly closed");
        }
    }

    async fn create_health_service_mock() -> (SocketAddr, UnboundedReceiver<()>, DropGuard) {
        let cancel = CancellationToken::new();
        let socket = TcpSocket::new_v4().unwrap();
        socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();

        let listen_addr = socket.local_addr().unwrap();
        let listener = TcpListenerStream::new(socket.listen(512).unwrap());

        let (request_tx, request_rx) = unbounded_channel();

        tokio::spawn({
            let cancel = cancel.clone();
            async move {
                let service = HealthServiceMock { requests: request_tx };
                Server::builder()
                    .add_service(HealthServer::new(service))
                    .serve_with_incoming_shutdown(listener, cancel.cancelled_owned())
                    .await
            }
        });

        (listen_addr, request_rx, cancel.drop_guard())
    }

    async fn create_client(resolver: ResolverMock) -> HealthClient<Channel> {
        create_client_with_address("http://localhost:1234", resolver).await
    }

    async fn create_client_with_address(address: &'static str, resolver: ResolverMock) -> HealthClient<Channel> {
        let uri = Uri::from_static(address);
        let channel = Builder::new(uri)
            .expect("failed to create a builder")
            .connections_per_host(NonZero::new(1).unwrap())
            .dns_refresh_interval(Duration::from_secs(1))
            .connect_with_resolve(Identity, resolver)
            .await
            .expect("connect failed");

        HealthClient::new(channel)
    }

    struct ResolverMock {
        expected_host: String,
        expected_port: u16,
        queue: VecDeque<(Vec<SocketAddr>, usize)>,
    }

    #[derive(Snafu, Debug)]
    enum ResolverMockError {
        UnexpectedHost,
        QueueEmpty,
    }

    #[async_trait::async_trait]
    impl Resolve for ResolverMock {
        type Error = ResolverMockError;

        async fn resolve(&mut self, host: &str, port: u16) -> Result<HashSet<SocketAddr>, Self::Error> {
            if host != self.expected_host || port != self.expected_port {
                return Err(ResolverMockError::UnexpectedHost);
            }

            if let Some((ref item, ref count)) = self.queue.front_mut() {
                let result = item.iter().copied().collect();
                if *count == 1 {
                    self.queue.pop_front();
                }

                Ok(result)
            } else {
                Err(ResolverMockError::QueueEmpty)
            }
        }
    }

    #[derive(Snafu, Debug)]
    struct IdentityError;

    struct Identity;

    impl Configure for Identity {
        type Error = IdentityError;

        fn configure(&mut self, endpoint: Endpoint) -> Result<Endpoint, Self::Error> {
            Ok(endpoint)
        }
    }

    struct HealthServiceMock {
        requests: UnboundedSender<()>,
    }

    #[async_trait::async_trait]
    impl Health for HealthServiceMock {
        type WatchStream = BoxStream<'static, Result<HealthCheckResponse, Status>>;

        async fn check(&self, _request: Request<HealthCheckRequest>) -> Result<Response<HealthCheckResponse>, Status> {
            let _ = self.requests.send(());
            Ok(Response::new(HealthCheckResponse::default()))
        }

        async fn watch(&self, _request: Request<HealthCheckRequest>) -> Result<Response<Self::WatchStream>, Status> {
            unimplemented!()
        }
    }
}
