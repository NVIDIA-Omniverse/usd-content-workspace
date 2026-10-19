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

use std::time::Duration;

use bytes::{Bytes, BytesMut};
use fred::{
    clients::{Pool, SubscriberClient},
    interfaces::{ClientLike, EventInterface, KeysInterface, LuaInterface, PubsubInterface},
    types::Message,
};
use futures::{future::try_join_all, stream::BoxStream};
use serde::{Deserialize, Serialize};
use snafu::IntoError;
use storage_service_metrics::{
    future::FutureExt,
    utils::{record_metric, record_metric_value},
};
use tracing::{debug, trace, warn};
use uuid::Uuid;

use crate::{
    backend::metadata::cache::{
        l2::{
            error::{Error, KeyDecodeError},
            metrics::Status,
        },
        types::{
            AnyKeyRef, Event, Namespace, PointReadKeyRef, PointReadValue, PointReadValueRef, QueryKeyRef, QueryValue,
            ValueOrToken,
        },
    },
    common::error::PrettyPrintErrorExt as _,
};

pub mod error;

mod metrics;

const KEY_TYPE_PREFIX_POINT_READ: &[u8] = b"p:";
const KEY_TYPE_PREFIX_QUERY: &[u8] = b"q:";

const EVENT_BACKLOG: usize = 1024;

/// This Redis client doesn't support the IFEQ argument in the SET command.
const SCRIPT_SET_IFEQ_PX: &str = "return redis.call(\"SET\", KEYS[1], ARGV[1], \"IFEQ\", ARGV[2], \"PX\", ARGV[3])";

/// We want to delete keys and publish events in a single call.
const SCRIPT_INVALIDATE: &str = "local reply = redis.call(\"DEL\", unpack(KEYS)); for idx, key in pairs(KEYS) do redis.call(\"PUBLISH\", ARGV[1], key) end; return reply";

/// Write slot claim token.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
pub struct ClaimToken(Uuid);

impl ClaimToken {
    fn new_random() -> Self {
        Self(Uuid::new_v4())
    }
}

#[derive(Debug)]
pub struct Cache {
    pool: Pool,
    key_prefix: String,
    invalidation_channel_name: String,
    cache_entry_ttl_ms: i64,
    claim_ttl_ms: i64,
    metrics: metrics::Metrics,
}

impl Cache {
    pub fn new(pool: Pool, key_prefix: impl Into<String>, cache_entry_ttl: Duration, claim_ttl: Duration) -> Self {
        let key_prefix = key_prefix.into();

        Self {
            cache_entry_ttl_ms: cache_entry_ttl.as_millis() as i64,
            claim_ttl_ms: claim_ttl.as_millis() as i64,
            invalidation_channel_name: format!("{key_prefix}invalidate"),
            key_prefix,
            metrics: metrics::Metrics::new(),
            pool,
        }
    }

    pub async fn invalidate_keys<'a>(&self, keys: impl IntoIterator<Item = AnyKeyRef<'a>> + 'a) -> Result<(), Error> {
        use error::error::Command;
        use fred::types::Value;

        let keys: Vec<_> = keys
            .into_iter()
            .map(|key| {
                let encoded_key = match key {
                    AnyKeyRef::PointRead(key) => encode_point_read_key(self.key_prefix.as_bytes(), key),
                    AnyKeyRef::Query(key) => encode_query_key(self.key_prefix.as_bytes(), key),
                };

                encoded_key
            })
            .collect();

        if keys.is_empty() {
            trace!("the keyset is empty, nothing to invalidate");
            return Ok(());
        }

        trace!(?keys, channel = ?self.invalidation_channel_name, "invalidating keys");

        let _: Value = self
            .pool
            .eval(SCRIPT_INVALIDATE, keys, [&self.invalidation_channel_name])
            .with_latency_metric(&self.metrics.invalidate_operation_latency)
            .await
            .inspect(record_metric_value(
                &self.metrics.invalidate_operation_results,
                Status::Ok,
            ))
            .inspect_err(record_metric(&self.metrics.invalidate_operation_results))
            .map_err(|source| Command.into_error(source))?;

        Ok(())
    }

    pub async fn read_point_read_values<'a>(
        &self,
        keys: impl IntoIterator<Item = PointReadKeyRef<'a>> + 'a,
    ) -> Result<Vec<ValueOrToken<PointReadValue, ClaimToken>>, Error> {
        // todo: use a batch operation or a batching script.
        // todo: individual errors shouldn't fail the entire batch.

        let results = try_join_all(keys.into_iter().map(|key| self.read_point_read_value(key))).await?;

        Ok(results)
    }

    pub async fn read_query_value(&self, key: QueryKeyRef<'_>) -> Result<ValueOrToken<QueryValue, ClaimToken>, Error> {
        use error::error::{Command, UnexpectedValueKind, ValueDecode};
        use fred::types::{Expiration, Key, SetOptions, Value};

        trace!(?key, "reading a query value");

        let token = ClaimToken::new_random();

        let encoded_key = encode_query_key(self.key_prefix.as_bytes(), key);
        let encoded_claim = encode_as_claimed(token);
        let result: Value = self
            .pool
            .set(
                Key::from(encoded_key),
                Value::Bytes(encoded_claim),
                Some(Expiration::PX(self.claim_ttl_ms)),
                Some(SetOptions::NX),
                true,
            )
            .with_latency_metric(&self.metrics.get_or_claim_operation_latency)
            .await
            .inspect(record_metric_value(
                &self.metrics.get_or_claim_operation_results,
                Status::Ok,
            ))
            .inspect_err(record_metric(&self.metrics.get_or_claim_operation_results))
            .map_err(|source| Command.into_error(source))?;

        let bytes = match result {
            Value::Null => return Ok(ValueOrToken::Token(token)),
            Value::Bytes(value) => value,
            Value::String(value) => value.into_inner(),
            v => return UnexpectedValueKind { kind: v.kind() }.fail(),
        };

        self.metrics.cache_access.add(1);

        let cache_entry: CacheValue<QueryValue> =
            serde_json::de::from_slice(&bytes).map_err(|source| ValueDecode.into_error(source))?;
        match cache_entry {
            CacheValue::Value(value) => Ok(ValueOrToken::Value(value)),
            CacheValue::Claimed(token) => {
                self.metrics.cache_miss.add(1);
                Ok(ValueOrToken::Token(token))
            },
        }
    }

    pub async fn write_point_read_values<'a>(
        &self,
        key_values: impl IntoIterator<Item = (PointReadKeyRef<'a>, PointReadValueRef<'a>, ClaimToken)> + 'a,
    ) -> Result<(), Error> {
        // todo: a single write must not fail the entire batch
        // todo: we could use request batching

        try_join_all(
            key_values
                .into_iter()
                .map(|(key, value, token)| self.write_point_read_value(key, value, token)),
        )
        .await?;
        Ok(())
    }

    pub async fn write_query_value(
        &self,
        key: QueryKeyRef<'_>,
        value: &QueryValue,
        token: ClaimToken,
    ) -> Result<(), Error> {
        trace!(?key, ?value, ?token, "writing a query value");

        let encoded_key = encode_query_key(self.key_prefix.as_bytes(), key);

        self.write_with_token(encoded_key, &value, token).await
    }

    pub async fn event_stream(&self) -> Result<BoxStream<'static, Event>, Error> {
        use async_stream::stream;
        use error::error::{Command, SubscriberInit};
        use tokio::sync::{broadcast::error::RecvError, mpsc::channel};
        use tokio_util::sync::CancellationToken;

        let client = SubscriberClient::new(
            self.pool.client_config().clone(),
            Some(self.pool.perf_config().clone()),
            Some(self.pool.connection_config().clone()),
            self.pool.client_reconnect_policy(),
        );

        debug!("initializing a subscriber client");
        client
            .init()
            .await
            .map_err(|source| SubscriberInit.into_error(source))?;

        #[allow(clippy::let_underscore_future)]
        let _ = client.manage_subscriptions();

        debug!(channel = ?self.invalidation_channel_name, "subscribing to keyspace events");
        client
            .subscribe(&self.invalidation_channel_name)
            .await
            .map_err(|source| Command.into_error(source))?;

        let (event_tx, mut event_rx) = channel(EVENT_BACKLOG);

        let cancel = CancellationToken::new();
        let guard = cancel.clone().drop_guard();
        tokio::spawn(cancel.clone().run_until_cancelled_owned({
            let client = client.clone();
            let event_tx = event_tx.clone();
            async move {
                let mut reconnect_rx = client.reconnect_rx();
                loop {
                    let maybe_server = reconnect_rx.recv().await;
                    let event = match maybe_server {
                        Ok(server) => {
                            debug!(?server, "reconnect event received");
                            Event::Reconnect
                        },
                        Err(RecvError::Lagged(skipped)) => {
                            warn!(?skipped, "reconnect event receiver lagged");
                            Event::Reconnect
                        },
                        Err(RecvError::Closed) => {
                            debug!("reconnect event sender closed");
                            return;
                        },
                    };

                    let _ = event_tx.send(event).await;
                }
            }
        }));

        tokio::spawn(cancel.run_until_cancelled_owned({
            let key_prefix = self.key_prefix.clone();
            async move {
                let mut message_rx = client.message_rx();
                loop {
                    let event = match message_rx.recv().await {
                        Ok(message) => {
                            let Ok(event) = decode_message(key_prefix.as_bytes(), message)
                                .inspect_err(|error| warn!(error = %error.pretty(), "failed to decode an event"))
                            else {
                                continue;
                            };

                            event
                        },
                        Err(RecvError::Lagged(skipped)) => {
                            warn!(?skipped, "message receiver lagged");
                            Event::Reconnect
                        },
                        Err(RecvError::Closed) => {
                            debug!("message sender closed");
                            return;
                        },
                    };

                    let _ = event_tx.send(event).await;
                }
            }
        }));

        Ok(Box::pin(stream! {
            while let Some(event) = event_rx.recv().await {
                yield event;
            }

            drop(guard);
        }))
    }

    async fn write_point_read_value(
        &self,
        key: PointReadKeyRef<'_>,
        value: PointReadValueRef<'_>,
        token: ClaimToken,
    ) -> Result<(), Error> {
        trace!(?key, ?value, ?token, "writing a point-read value");

        let encoded_key = encode_point_read_key(self.key_prefix.as_bytes(), key);

        self.write_with_token(encoded_key, &value, token).await
    }

    async fn read_point_read_value(
        &self,
        key: PointReadKeyRef<'_>,
    ) -> Result<ValueOrToken<PointReadValue, ClaimToken>, Error> {
        use error::error::{Command, UnexpectedValueKind, ValueDecode};
        use fred::types::{Expiration, Key, SetOptions, Value};

        trace!(?key, "reading a point-read value");

        let token = ClaimToken::new_random();

        let encoded_key = encode_point_read_key(self.key_prefix.as_bytes(), key);
        let encoded_token = encode_as_claimed(token);

        let result: Value = self
            .pool
            .set(
                Key::from(encoded_key),
                Value::from(encoded_token),
                Some(Expiration::PX(self.claim_ttl_ms)),
                Some(SetOptions::NX),
                true,
            )
            .with_latency_metric(&self.metrics.get_or_claim_operation_latency)
            .await
            .inspect(record_metric_value(
                &self.metrics.get_or_claim_operation_results,
                Status::Ok,
            ))
            .inspect_err(record_metric(&self.metrics.get_or_claim_operation_results))
            .map_err(|source| Command.into_error(source))?;

        let bytes = match result {
            Value::Bytes(value) => value,
            Value::String(value) => value.into_inner(),
            Value::Null => return Ok(ValueOrToken::Token(token)),
            v => return UnexpectedValueKind { kind: v.kind() }.fail(),
        };

        self.metrics.cache_access.add(1);

        let cache_entry: CacheValue<PointReadValue> =
            serde_json::de::from_slice(&bytes).map_err(|source| ValueDecode.into_error(source))?;
        match cache_entry {
            CacheValue::Value(value) => Ok(ValueOrToken::Value(value)),
            CacheValue::Claimed(token) => {
                self.metrics.cache_miss.add(1);
                Ok(ValueOrToken::Token(token))
            },
        }
    }

    async fn write_with_token(
        &self,
        encoded_key: Bytes,
        value: &impl Serialize,
        token: ClaimToken,
    ) -> Result<(), Error> {
        use error::error::{Command, ValueEncode};
        use fred::types::{Key, Value};

        let encoded_claim = encode_as_claimed(token);
        let encoded_value =
            serde_json::ser::to_vec(&CacheValueRef::Value(value)).map_err(|source| ValueEncode.into_error(source))?;

        let _result: Value = self
            .pool
            .eval(
                SCRIPT_SET_IFEQ_PX,
                Key::from(encoded_key),
                [
                    Value::Bytes(Bytes::from(encoded_value)),
                    Value::Bytes(encoded_claim),
                    Value::Integer(self.cache_entry_ttl_ms),
                ],
            )
            .with_latency_metric(&self.metrics.write_operation_latency)
            .await
            .inspect(record_metric_value(&self.metrics.write_operation_results, Status::Ok))
            .inspect_err(record_metric(&self.metrics.write_operation_results))
            .map_err(|source| Command.into_error(source))?;

        Ok(())
    }
}

#[derive(Deserialize, Debug)]
enum CacheValue<T> {
    #[serde(rename = "claimed")]
    Claimed(ClaimToken),
    #[serde(rename = "value")]
    Value(T),
}

#[derive(Serialize, Debug)]
enum CacheValueRef<'a, T> {
    #[serde(rename = "claimed")]
    _Claimed(&'a ClaimToken),
    #[serde(rename = "value")]
    Value(&'a T),
}

fn encode_as_claimed(value: ClaimToken) -> Bytes {
    let value = format!(r#"{{"claimed":"{}"}}"#, value.0);
    Bytes::from(value)
}

fn encode_point_read_key(prefix: &[u8], key: PointReadKeyRef<'_>) -> Bytes {
    encode_any_key(prefix, KEY_TYPE_PREFIX_POINT_READ, AnyKeyRef::PointRead(key))
}

fn encode_query_key(prefix: &[u8], key: QueryKeyRef<'_>) -> Bytes {
    encode_any_key(prefix, KEY_TYPE_PREFIX_QUERY, AnyKeyRef::Query(key))
}

fn encode_any_key(prefix: &[u8], key_type_prefix: &[u8], key: AnyKeyRef<'_>) -> Bytes {
    let (resource_uri, namespace, maybe_metadata_key) = match key {
        AnyKeyRef::Query(ref key) => (key.resource_uri.as_bytes(), key.namespace.as_str().as_bytes(), None),
        AnyKeyRef::PointRead(ref key) => (
            key.resource_uri.as_bytes(),
            key.namespace.as_str().as_bytes(),
            Some(key.metadata_key.as_bytes()),
        ),
    };

    let resource_uri_len_bytes = (resource_uri.len() as u32).to_le_bytes();
    let namespace_len_bytes = (namespace.len() as u32).to_le_bytes();
    let maybe_metadata_key_and_len = maybe_metadata_key.map(|metadata_key| {
        let metadata_key_len_bytes = (metadata_key.len() as u32).to_le_bytes();
        (metadata_key, metadata_key_len_bytes)
    });

    let mut buffer = BytesMut::with_capacity(
        prefix.len()
            + key_type_prefix.len()
            + resource_uri_len_bytes.len()
            + resource_uri.len()
            + namespace_len_bytes.len()
            + namespace.len()
            + maybe_metadata_key_and_len
                .map(|(key, key_len_bytes)| key_len_bytes.len() + key.len())
                .unwrap_or_default(),
    );
    buffer.extend_from_slice(prefix);
    buffer.extend_from_slice(key_type_prefix);
    buffer.extend_from_slice(&resource_uri_len_bytes);
    buffer.extend_from_slice(resource_uri);
    buffer.extend_from_slice(&namespace_len_bytes);
    buffer.extend_from_slice(namespace);

    if let Some((metadata_key, metadata_key_len_bytes)) = maybe_metadata_key_and_len {
        buffer.extend_from_slice(&metadata_key_len_bytes);
        buffer.extend_from_slice(metadata_key);
    }

    buffer.freeze()
}

fn decode_any_key<'a>(prefix: &[u8], value: &'a [u8]) -> Result<AnyKeyRef<'a>, KeyDecodeError> {
    use error::key_decode_error::UnexpectedPrefix;

    let Some(suffix) = value.strip_prefix(prefix) else {
        return UnexpectedPrefix.fail();
    };

    if let Some(suffix) = suffix.strip_prefix(KEY_TYPE_PREFIX_POINT_READ) {
        decode_point_read_key(suffix).map(AnyKeyRef::PointRead)
    } else if let Some(suffix) = suffix.strip_prefix(KEY_TYPE_PREFIX_QUERY) {
        decode_query_key(suffix).map(AnyKeyRef::Query)
    } else {
        UnexpectedPrefix.fail()
    }
}

fn decode_point_read_key(value: &[u8]) -> Result<PointReadKeyRef<'_>, KeyDecodeError> {
    use error::key_decode_error::BufferNotExhausted;

    let (resource_uri, suffix) = decode_str(value)?;
    let (namespace, suffix) = decode_str(suffix)?;
    let (metadata_key, suffix) = decode_str(suffix)?;

    if !suffix.is_empty() {
        return BufferNotExhausted.fail();
    }

    Ok(PointReadKeyRef {
        resource_uri,
        metadata_key,
        namespace: Namespace::from_value(namespace),
    })
}

fn decode_query_key(value: &[u8]) -> Result<QueryKeyRef<'_>, KeyDecodeError> {
    use error::key_decode_error::BufferNotExhausted;

    let (resource_uri, suffix) = decode_str(value)?;
    let (namespace, suffix) = decode_str(suffix)?;

    if !suffix.is_empty() {
        return BufferNotExhausted.fail();
    }

    Ok(QueryKeyRef {
        resource_uri,
        namespace: Namespace::from_value(namespace),
    })
}

fn decode_str(value: &[u8]) -> Result<(&str, &[u8]), KeyDecodeError> {
    use error::key_decode_error::{BufferTooShort, Utf8Decode};

    let Some((prefix, suffix)) = value.split_at_checked(4) else {
        return BufferTooShort.fail();
    };

    let string_len = u32::from_le_bytes([prefix[0], prefix[1], prefix[2], prefix[3]]);
    let Some((string, suffix)) = suffix.split_at_checked(string_len as usize) else {
        return BufferTooShort.fail();
    };

    let value = str::from_utf8(string).map_err(|source| Utf8Decode.into_error(source))?;

    Ok((value, suffix))
}

fn decode_message(key_prefix: &[u8], message: Message) -> Result<Event, Error> {
    use error::error::{KeyDecode, UnexpectedValueKind};
    use fred::types::Value;

    let bytes = match message.value {
        Value::String(v) => v.into_inner(),
        Value::Bytes(v) => v,
        v => return UnexpectedValueKind { kind: v.kind() }.fail(),
    };

    let key = decode_any_key(key_prefix, &bytes).map_err(|source| KeyDecode.into_error(source))?;
    trace!(?key, "keyspace event decoded");

    Ok(match key {
        AnyKeyRef::PointRead(key) => Event::PointReadKeyInvalidate(key.to_owned()),
        AnyKeyRef::Query(key) => Event::QueryKeyInvalidate(key.to_owned()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use assert_matches::assert_matches;
    use rstest::rstest;

    async fn create_cache() -> Cache {
        use fred::{
            clients::Pool,
            types::config::{Config, ServerConfig},
        };
        use std::env::var;

        let redis_host = var("REDIS_HOST").unwrap();
        let redis_port = var("REDIS_PORT").unwrap();

        let pool = Pool::new(
            Config {
                server: ServerConfig::new_centralized(redis_host, redis_port.parse().unwrap()),
                ..Default::default()
            },
            None,
            None,
            None,
            1,
        )
        .unwrap();
        pool.init().await.unwrap();

        Cache::new(
            pool,
            uuid::Uuid::new_v4().to_string(),
            Duration::from_hours(1),
            Duration::from_hours(1),
        )
    }

    #[test]
    fn test_encode_as_claimed_deserializes_as_cache_entry() {
        use storage_service_protos::nvidia::omniverse::storage::metadata::v1alpha::UserMetadataValue;

        let token = ClaimToken::new_random();
        let encoded_token = encode_as_claimed(token);

        let decoded_message: CacheValue<UserMetadataValue> = serde_json::de::from_slice(&encoded_token).unwrap();

        assert_matches!(decoded_message, CacheValue::Claimed(t) if t == token);
    }

    #[rstest]
    #[case(
        Namespace::Default,
        b"bytes:p:\x02\x00\x00\x00aa\x07\x00\x00\x00default\x04\x00\x00\x00cccc"
    )]
    #[case(
        Namespace::Explicit("bbb"),
        b"bytes:p:\x02\x00\x00\x00aa\x03\x00\x00\x00bbb\x04\x00\x00\x00cccc"
    )]
    fn test_encode_point_read_key(#[case] namespace: Namespace<&str>, #[case] expected: &[u8]) {
        let encoded = encode_point_read_key(
            b"bytes:",
            PointReadKeyRef {
                resource_uri: "aa",
                metadata_key: "cccc",
                namespace,
            },
        );
        assert_eq!(encoded, expected);
    }

    #[rstest]
    #[case(Namespace::Default, b"bytes:q:\x02\x00\x00\x00aa\x07\x00\x00\x00default")]
    #[case(Namespace::Explicit("bbb"), b"bytes:q:\x02\x00\x00\x00aa\x03\x00\x00\x00bbb")]
    fn test_encode_query_key(#[case] namespace: Namespace<&str>, #[case] expected: &[u8]) {
        let encoded = encode_query_key(
            b"bytes:",
            QueryKeyRef {
                resource_uri: "aa",
                namespace,
            },
        );
        assert_eq!(encoded, expected);
    }

    #[test]
    fn test_decode_point_read_key() {
        let decoded = decode_any_key(
            b"prefix:",
            b"prefix:p:\x02\x00\x00\x00aa\x01\x00\x00\x00b\x03\x00\x00\x00cde",
        )
        .unwrap();
        assert_eq!(
            decoded,
            AnyKeyRef::PointRead(PointReadKeyRef {
                resource_uri: "aa",
                namespace: Namespace::Explicit("b"),
                metadata_key: "cde",
            })
        );
    }

    #[test]
    fn test_decode_query_key() {
        let decoded = decode_any_key(b"prefix:", b"prefix:q:\x02\x00\x00\x00aa\x01\x00\x00\x00b").unwrap();
        assert_eq!(
            decoded,
            AnyKeyRef::Query(QueryKeyRef {
                resource_uri: "aa",
                namespace: Namespace::Explicit("b")
            })
        );
    }

    #[rstest]
    #[case(b"prefix:c:\x01\x00\x00\x00a\x02\x00\x00\x00bb\x03\x00\x00\x00cc")]
    #[case(b"prefix:p:\x01\x00\x00\x00a\xff\xff\xff\x00abgd")]
    #[case(b"prefix:p:\x01\x00\x00\x00a\x02\x00\x00\x00bb\x03\x00\x00\x00cc")]
    #[case(b"prefix:p:\x01\x00\x00\x01a\x02\x00\x00\x00bb\x03\x00\x00\x00ccc\x00")]
    #[case(b"prefixp:\x01\x00\x00\x00a\x02\x00\x00\x00bb\x03\x00\x00\x00ccc")]
    #[case(b"prefix:q:\x01\x00\x00\x00a\x02\x00\x00\x00bb\x03\x00\x00\x00ccc")]
    #[case(b"prefix:q:\x01\x00\x00\x00a\x02\x00\x00\x00\xff\xff")]
    fn test_decode_any_key_error(#[case] value: &[u8]) {
        assert_matches!(decode_any_key(b"prefix:", value), Err(_));
    }

    #[test_log::test(tokio::test)]
    async fn test_read_point_read_values_claims_a_slot() {
        let cache = create_cache().await;
        let key = PointReadKeyRef {
            metadata_key: "key",
            namespace: Namespace::Default,
            resource_uri: "https://myhost.mydomain/resource-0001",
        };

        let results = cache.read_point_read_values([key]).await.unwrap();

        assert_eq!(results.len(), 1);
        assert_matches!(results[0], ValueOrToken::Token(_));
    }

    #[test_log::test(tokio::test)]
    async fn test_read_point_read_values_returns_existing_token() {
        let cache = create_cache().await;
        let key = PointReadKeyRef {
            metadata_key: "key",
            namespace: Namespace::Default,
            resource_uri: "https://myhost.mydomain/resource-0002",
        };

        let results_1 = cache.read_point_read_values([key]).await.unwrap();

        assert_eq!(results_1.len(), 1);
        assert_matches!(results_1[0], ValueOrToken::Token(_));

        let results_2 = cache.read_point_read_values([key]).await.unwrap();

        assert_eq!(results_2.len(), 1);
        assert_matches!((&results_1[0], &results_2[0]), (ValueOrToken::Token(t1), ValueOrToken::Token(t2)) if t1 == t2);
    }

    #[test_log::test(tokio::test)]
    async fn test_read_query_value_claims_a_slot() {
        let cache = create_cache().await;
        let key = QueryKeyRef {
            namespace: Namespace::Default,
            resource_uri: "https://myhost.mydomain/resource-0003",
        };

        let result = cache.read_query_value(key).await.unwrap();

        assert_matches!(result, ValueOrToken::Token(_));
    }

    #[test_log::test(tokio::test)]
    async fn test_read_query_value_returns_existing_token() {
        let cache = create_cache().await;
        let key = QueryKeyRef {
            namespace: Namespace::Default,
            resource_uri: "https://myhost.mydomain/resource-0004",
        };

        let result_1 = cache.read_query_value(key).await.unwrap();

        assert_matches!(result_1, ValueOrToken::Token(_));

        let result_2 = cache.read_query_value(key).await.unwrap();

        assert_matches!((result_1, result_2), (ValueOrToken::Token(t1), ValueOrToken::Token(t2)) if t1 == t2);
    }

    #[test_log::test(tokio::test)]
    async fn test_write_query_value() {
        let cache = create_cache().await;
        let value = QueryValue::default();
        let key = QueryKeyRef {
            namespace: Namespace::Default,
            resource_uri: "https://myhost.mydomain/resource-0005",
        };

        let token = cache.read_query_value(key).await.unwrap().unwrap_token();

        cache.write_query_value(key, &value, token).await.unwrap();

        let read_result = cache.read_query_value(key).await.unwrap();

        assert_matches!(read_result, ValueOrToken::Value(v) if v == value);
    }

    #[test_log::test(tokio::test)]
    async fn test_write_query_value_after_invalidate() {
        let cache = create_cache().await;
        let value = QueryValue::default();
        let key = QueryKeyRef {
            namespace: Namespace::Default,
            resource_uri: "https://myhost.mydomain/resource-0006",
        };

        let token = cache.read_query_value(key).await.unwrap().unwrap_token();

        cache.invalidate_keys([AnyKeyRef::Query(key)]).await.unwrap();

        cache.write_query_value(key, &value, token).await.unwrap();

        let read_result = cache.read_query_value(key).await.unwrap();

        assert_matches!(read_result, ValueOrToken::Token(t) if t != token);
    }

    #[test_log::test(tokio::test)]
    async fn test_write_point_read_value() {
        let cache = create_cache().await;
        let key = PointReadKeyRef {
            metadata_key: "key",
            namespace: Namespace::Default,
            resource_uri: "https://myhost.mydomain/resource-0007",
        };

        let token = cache.read_point_read_value(key).await.unwrap().unwrap_token();

        cache
            .write_point_read_values([(key, PointReadValueRef::Missing, token)])
            .await
            .unwrap();

        let read_result = cache.read_point_read_value(key).await.unwrap();

        assert_matches!(read_result, ValueOrToken::Value(PointReadValue::Missing));
    }

    #[test_log::test(tokio::test)]
    async fn test_write_point_read_value_after_invalidate() {
        let cache = create_cache().await;
        let key = PointReadKeyRef {
            metadata_key: "key",
            namespace: Namespace::Default,
            resource_uri: "https://myhost.mydomain/resource-0008",
        };

        let token = cache.read_point_read_value(key).await.unwrap().unwrap_token();

        cache.invalidate_keys([AnyKeyRef::PointRead(key)]).await.unwrap();

        cache
            .write_point_read_values([(key, PointReadValueRef::Missing, token)])
            .await
            .unwrap();

        let read_result = cache.read_point_read_value(key).await.unwrap();

        assert_matches!(read_result, ValueOrToken::Token(t) if t != token);
    }
}
