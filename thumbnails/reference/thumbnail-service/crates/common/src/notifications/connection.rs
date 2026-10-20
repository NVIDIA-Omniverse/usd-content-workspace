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

//! gRPC connection management for the event-consumer service.
//!
//! The channel is rebuilt on every reconnect, but the [`AuthInterceptor`] and
//! [`TokenRefresher`] are reused across reconnects (so 401-recovery has a
//! stable refresh target and tokens don't get thrown away by transport churn).

use std::sync::Arc;
use std::time::Duration;

use thumbnail_auth::TokenRefresher;
use tracing::info;

use notifications_proto::nvidia::omniverse::notifications::consumer::v1beta::event_consumer_service_client::EventConsumerServiceClient;

use super::{EventConsumer, EventConsumerConfig};

impl EventConsumer {
    pub fn new(config: EventConsumerConfig) -> Self {
        Self {
            config,
            grpc_client: None,
        }
    }

    pub async fn connect_grpc(&mut self) -> Result<(), String> {
        let endpoint_str = if self.config.endpoint.starts_with("http://")
            || self.config.endpoint.starts_with("https://")
        {
            self.config.endpoint.clone()
        } else {
            format!("http://{}", self.config.endpoint)
        };

        info!(
            "[PLUGIN] Connecting to gRPC notification service at: {}",
            endpoint_str
        );

        // The notifications stream is intentionally long-lived, so keepalive
        // settings matter more than per-request timeouts here.
        let channel = tonic::transport::Endpoint::new(endpoint_str.clone())
            .map_err(|e| format!("Invalid notification endpoint {}: {}", endpoint_str, e))?
            .connect_timeout(Duration::from_secs(10))
            .http2_keep_alive_interval(Duration::from_secs(300))
            .keep_alive_timeout(Duration::from_secs(20))
            .keep_alive_while_idle(true)
            .tcp_keepalive(Some(Duration::from_secs(300)))
            .connect()
            .await
            .map_err(|e| {
                format!(
                    "Failed to connect to notification service at {}: {}",
                    endpoint_str, e
                )
            })?;

        info!("[OK] gRPC channel established with keepalive settings");
        info!("   • Connection lifecycle: until UNAUTHENTICATED (reactive token refresh); other stream errors back off");
        info!("   • Keepalive: 300s interval, 20s timeout, TCP keepalive 300s");

        self.grpc_client = Some(EventConsumerServiceClient::with_interceptor(
            channel,
            self.config.interceptor.clone(),
        ));

        info!("[OK] Connected to gRPC notification service");
        Ok(())
    }

    pub(crate) fn refresher(&self) -> &Arc<dyn TokenRefresher> {
        &self.config.refresher
    }

    #[cfg(test)]
    pub fn get_refresher(&self) -> Arc<dyn TokenRefresher> {
        self.config.refresher.clone()
    }
}
