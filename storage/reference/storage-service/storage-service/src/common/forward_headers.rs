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

use std::sync::Arc;

use http::{
    header::{AUTHORIZATION, CONNECTION, CONTENT_LENGTH, CONTENT_TYPE, HOST, TE, TRANSFER_ENCODING, UPGRADE},
    HeaderMap, HeaderName,
};
use serde::{de::Error as _, Deserialize};
use snafu::{Location, OptionExt as _, Snafu};

/// Headers the service produces itself or that the transports treat specially. Forwarding
/// them would either duplicate the outbound value or corrupt the downstream request.
const RESERVED_HEADER_NAMES: [HeaderName; 10] = [
    AUTHORIZATION,
    CONNECTION,
    CONTENT_LENGTH,
    CONTENT_TYPE,
    HOST,
    TE,
    TRANSFER_ENCODING,
    UPGRADE,
    HeaderName::from_static("traceparent"),
    HeaderName::from_static("tracestate"),
];

const GRPC_HEADER_PREFIX: &str = "grpc-";
const GRPC_BINARY_HEADER_SUFFIX: &str = "-bin";

/// Names of the incoming request headers that are copied onto downstream requests.
#[derive(Debug, Clone, Default)]
pub struct ForwardedHeaderNames {
    names: Arc<[HeaderName]>,
}

impl ForwardedHeaderNames {
    /// Validate the header names. Names are normalized to lowercase and deduplicated.
    pub fn try_from_iter<I, S>(names: I) -> Result<Self, InvalidHeaderName>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut validated = Vec::new();
        for name in names {
            let name = name.as_ref();
            let header_name = HeaderName::from_bytes(name.as_bytes())
                .ok()
                .context(MalformedSnafu { name })?;

            if RESERVED_HEADER_NAMES.contains(&header_name) {
                return ReservedSnafu { name }.fail();
            }
            if header_name.as_str().starts_with(GRPC_HEADER_PREFIX) {
                return ReservedGrpcSnafu { name }.fail();
            }
            if header_name.as_str().ends_with(GRPC_BINARY_HEADER_SUFFIX) {
                return BinaryGrpcSnafu { name }.fail();
            }

            if !validated.contains(&header_name) {
                validated.push(header_name);
            }
        }

        Ok(Self {
            names: validated.into(),
        })
    }

    /// Iterate over the configured header names.
    pub fn iter(&self) -> impl Iterator<Item = &HeaderName> {
        self.names.iter()
    }

    /// Collect the configured headers present in the incoming request metadata. The returned map is
    /// empty, without allocating, when nothing is configured or none of the headers are present.
    pub fn collect(&self, metadata: &HeaderMap) -> HeaderMap {
        if self.names.is_empty() {
            return Default::default();
        }

        let mut result = HeaderMap::with_capacity(self.names.len().min(metadata.len()));
        for key in self.names.iter() {
            for value in metadata.get_all(key) {
                result.append(key.clone(), value.clone());
            }
        }

        result
    }
}

impl<'de> Deserialize<'de> for ForwardedHeaderNames {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let names = Vec::<String>::deserialize(deserializer)?;
        Self::try_from_iter(names).map_err(D::Error::custom)
    }
}

/// A header name that cannot be forwarded.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum InvalidHeaderName {
    #[snafu(display("header {name:?} is not a valid header name"))]
    Malformed {
        name: String,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("header {name:?} cannot be forwarded: reserved header"))]
    Reserved {
        name: String,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("header {name:?} cannot be forwarded: reserved grpc header"))]
    ReservedGrpc {
        name: String,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("header {name:?} cannot be forwarded: binary grpc metadata cannot be forwarded"))]
    BinaryGrpc {
        name: String,
        #[snafu(provide, implicit)]
        location: Location,
    },
}

#[cfg(test)]
mod tests {
    use http::{HeaderName, HeaderValue};
    use rstest::rstest;
    use tonic::metadata::MetadataMap;

    use super::ForwardedHeaderNames;

    #[test]
    fn test_normalizes_and_deduplicates_names() {
        let names = ForwardedHeaderNames::try_from_iter(["X-Request-Id", "user-id", "x-request-id"]).unwrap();
        let names: Vec<_> = names.iter().map(HeaderName::as_str).collect();
        assert_eq!(names, ["x-request-id", "user-id"]);
    }

    #[rstest]
    #[case("authorization")]
    #[case("Authorization")]
    #[case("content-type")]
    #[case("traceparent")]
    #[case("grpc-timeout")]
    #[case("custom-bin")]
    #[case("not a header")]
    #[case("")]
    fn test_rejects_reserved_and_invalid_names(#[case] name: &str) {
        assert!(ForwardedHeaderNames::try_from_iter([name]).is_err());
    }

    #[test]
    fn test_deserializes_from_a_string_list() {
        let names: ForwardedHeaderNames = serde_json::from_str(r#"["user-id", "X-Forwarded-For"]"#).unwrap();
        let names: Vec<_> = names.iter().map(HeaderName::as_str).collect();
        assert_eq!(names, ["user-id", "x-forwarded-for"]);

        let error = serde_json::from_str::<ForwardedHeaderNames>(r#"["authorization"]"#).unwrap_err();
        assert!(error.to_string().contains("reserved header"), "{error}");
    }

    #[test]
    fn test_collects_all_values_of_configured_headers() {
        let names = ForwardedHeaderNames::try_from_iter(["user-id", "x-request-id"]).unwrap();
        let mut metadata = MetadataMap::new();
        metadata.append("user-id", "alice".parse().unwrap());
        metadata.append("user-id", "bob".parse().unwrap());
        metadata.append("authorization", "Bearer token".parse().unwrap());

        let forwarded = names.collect(metadata.as_ref());
        let user_ids: Vec<_> = forwarded.get_all("user-id").iter().collect();
        assert_eq!(
            user_ids,
            [HeaderValue::from_static("alice"), HeaderValue::from_static("bob")]
        );
        assert!(!forwarded.contains_key("authorization"));
        assert!(!forwarded.contains_key("x-request-id"));
    }

    #[test]
    fn test_empty_configuration_collects_nothing() {
        let mut metadata = MetadataMap::new();
        metadata.append("user-id", "alice".parse().unwrap());
        assert!(ForwardedHeaderNames::default().collect(metadata.as_ref()).is_empty());
    }
}
