// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use fred::clients::SubscriberClient;
use fred::error::{Error as RedisError, ErrorKind};
use fred::interfaces::{ClientLike, EventInterface, PubsubInterface};
use fred::types::Message;
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};

use super::expiry::CacheEntry;
use crate::observability::metrics::BoundUIntCounter;
use crate::warn_limited;

const SUBSCRIBER_START_ATTEMPTS: usize = 5;
const SUBSCRIBER_START_RETRY_BASE_DELAY_MS: u64 = 50;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InvalidationMessage {
    Key { key: String },
    Prefix { prefix: String },
}

impl InvalidationMessage {
    pub fn encode(&self) -> Result<Vec<u8>, postcard::Error> {
        let (kind, value) = match self {
            Self::Key { key } => ("key", key.as_str()),
            Self::Prefix { prefix } => ("prefix", prefix.as_str()),
        };
        postcard::to_allocvec(&(kind, value))
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, postcard::Error> {
        let (kind, value): (String, String) = postcard::from_bytes(bytes)?;
        match kind.as_str() {
            "key" => Ok(Self::Key { key: value }),
            "prefix" => Ok(Self::Prefix { prefix: value }),
            _ => Err(postcard::Error::SerdeDeCustom),
        }
    }
}

pub(crate) struct PubSubHandle {
    subscriber: SubscriberClient,
    task: JoinHandle<Result<(), fred::error::Error>>,
    watchdog: Option<JoinHandle<()>>,
}

#[derive(Clone)]
pub(crate) struct PubSubMetrics {
    message_dropped: BoundUIntCounter,
    reconnect: BoundUIntCounter,
}

impl PubSubMetrics {
    pub(crate) fn new(message_dropped: BoundUIntCounter, reconnect: BoundUIntCounter) -> Self {
        Self {
            message_dropped,
            reconnect,
        }
    }

    fn record_message_dropped(&self, count: u64) {
        self.message_dropped.add(count);
    }

    fn record_reconnect(&self) {
        self.reconnect.add(1);
    }
}

impl PubSubHandle {
    pub async fn shutdown(self) {
        if let Some(watchdog) = self.watchdog {
            watchdog.abort();
            let _ = watchdog.await;
        }
        let _ = self.subscriber.quit().await;
        let _ = self.task.await;
    }

    #[cfg(test)]
    pub(crate) fn connection_ids(&self) -> Vec<i64> {
        use fred::interfaces::ClientInterface;

        self.subscriber.connection_ids().values().copied().collect()
    }
}

pub(crate) async fn spawn_subscriber<V>(
    redis: &fred::clients::Client,
    channel: String,
    l1: moka::future::Cache<String, CacheEntry<V>>,
    local_generation: Arc<AtomicU64>,
    metrics: Option<PubSubMetrics>,
) -> Result<PubSubHandle, super::error::Error>
where
    V: Clone + Send + Sync + 'static,
{
    for attempt in 1..=SUBSCRIBER_START_ATTEMPTS {
        match try_spawn_subscriber(
            redis,
            &channel,
            l1.clone(),
            Arc::clone(&local_generation),
            metrics.clone(),
        )
        .await
        {
            Ok(handle) => return Ok(handle),
            Err(error) if attempt < SUBSCRIBER_START_ATTEMPTS && is_retryable_startup_error(&error) => {
                warn_limited!(
                    error = %error,
                    attempt,
                    max_attempts = SUBSCRIBER_START_ATTEMPTS,
                    "pubsub: subscriber startup failed; retrying"
                );
                sleep(Duration::from_millis(
                    SUBSCRIBER_START_RETRY_BASE_DELAY_MS * u64::try_from(attempt).unwrap_or(1),
                ))
                .await;
            },
            Err(error) => return Err(error.into()),
        }
    }

    unreachable!("subscriber startup retry loop always returns")
}

fn is_retryable_startup_error(error: &RedisError) -> bool {
    matches!(
        error.kind(),
        ErrorKind::IO | ErrorKind::Canceled | ErrorKind::Timeout | ErrorKind::Unknown | ErrorKind::Backpressure
    )
}

async fn try_spawn_subscriber<V>(
    redis: &fred::clients::Client,
    channel: &str,
    l1: moka::future::Cache<String, CacheEntry<V>>,
    local_generation: Arc<AtomicU64>,
    metrics: Option<PubSubMetrics>,
) -> Result<PubSubHandle, RedisError>
where
    V: Clone + Send + Sync + 'static,
{
    let subscriber = SubscriberClient::new(
        redis.client_config(),
        Some(redis.perf_config()),
        Some(redis.connection_config().clone()),
        redis.client_reconnect_policy(),
    );
    subscriber.init().await?;

    // On reconnect: invalidations may have been missed while disconnected, so
    // bump generation (poison any in-flight L2 fills) and clear L1 wholesale.
    // The bump must happen before invalidate_all so a concurrent L2 reader's
    // re-snapshot will see the advance.
    let l1_reconnect = l1.clone();
    let gen_reconnect = Arc::clone(&local_generation);
    let metrics_reconnect = metrics.clone();
    subscriber.on_reconnect(move |_| {
        let l1 = l1_reconnect.clone();
        let gen = Arc::clone(&gen_reconnect);
        let metrics = metrics_reconnect.clone();
        async move {
            if let Some(metrics) = metrics.as_ref() {
                metrics.record_reconnect();
            }
            gen.fetch_add(1, Ordering::Release);
            l1.invalidate_all();
            Ok(())
        }
    });

    let l1_msg = l1;
    let gen_msg = Arc::clone(&local_generation);
    let metrics_msg = metrics.clone();
    let mut messages = subscriber.message_rx();
    let task = tokio::spawn(async move {
        loop {
            let msg: Message = match messages.recv().await {
                Ok(msg) => msg,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    gen_msg.fetch_add(1, Ordering::Release);
                    l1_msg.invalidate_all();
                    if let Some(metrics) = metrics_msg.as_ref() {
                        metrics.record_message_dropped(skipped);
                    }
                    warn_limited!(skipped, "pubsub: listener lagged; clearing L1");
                    continue;
                },
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    break Ok(());
                },
            };
            let l1 = l1_msg.clone();
            let gen = Arc::clone(&gen_msg);
            // Try bytes first (new binary format); fall back to a string read
            // so a late roll-out doesn't lose messages from older publishers.
            let bytes: Vec<u8> = match msg.value.clone().convert::<Vec<u8>>() {
                Ok(v) => v,
                Err(_) => match msg.value.convert::<String>() {
                    Ok(s) => s.into_bytes(),
                    Err(_) => {
                        if let Some(metrics) = metrics_msg.as_ref() {
                            metrics.record_message_dropped(1);
                        }
                        warn_limited!("pubsub: dropping message with non-binary payload");
                        continue;
                    },
                },
            };
            let inv = match InvalidationMessage::decode(&bytes) {
                Ok(v) => v,
                Err(e) => {
                    if let Some(metrics) = metrics_msg.as_ref() {
                        metrics.record_message_dropped(1);
                    }
                    warn_limited!(error = %e, "pubsub: dropping malformed invalidation message");
                    continue;
                },
            };
            // Bump generation first so any L2 reader that snapshotted prior
            // to this message will refuse to backfill its L1 below.
            gen.fetch_add(1, Ordering::Release);
            match inv {
                InvalidationMessage::Key { key } => {
                    l1.invalidate(&key).await;
                },
                InvalidationMessage::Prefix { prefix } => {
                    if let Err(e) = l1.invalidate_entries_if(move |k, _| k.starts_with(&prefix)) {
                        warn_limited!(error = %e, "pubsub: invalidate_entries_if predicate cap exceeded");
                    }
                },
            }
        }
    });

    let watchdog = subscriber.manage_subscriptions();
    if let Err(error) = subscriber.subscribe(channel).await {
        watchdog.abort();
        task.abort();
        let _ = subscriber.quit().await;
        let _ = task.await;
        return Err(error);
    }

    Ok(PubSubHandle {
        subscriber,
        task,
        watchdog: Some(watchdog),
    })
}

#[cfg(test)]
mod tests {
    use super::InvalidationMessage;

    #[test]
    fn invalidation_message_round_trips_key() {
        let msg = InvalidationMessage::Key { key: "k".to_string() };
        let bytes = msg.encode().expect("encode");

        assert_eq!(InvalidationMessage::decode(&bytes).expect("decode"), msg);
    }

    #[test]
    fn invalidation_message_round_trips_prefix() {
        let msg = InvalidationMessage::Prefix {
            prefix: "folder/".to_string(),
        };
        let bytes = msg.encode().expect("encode");

        assert_eq!(InvalidationMessage::decode(&bytes).expect("decode"), msg);
    }
}
