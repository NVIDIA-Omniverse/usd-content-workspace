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

use indexmap::IndexMap;
use moka::Equivalent;
use serde::{Deserialize, Serialize};

use crate::backend::metadata::{ResourceLocation, UserMetadataValue, DEFAULT_NAMESPACE_NAME};

pub type QueryValue = IndexMap<String, UserMetadataValue>;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ValueOrToken<V, T> {
    Value(V),
    Token(T),
}

#[cfg(test)]
impl<V, T> ValueOrToken<V, T> {
    #[track_caller]
    pub fn unwrap_token(self) -> T {
        match self {
            Self::Value(_) => panic!("called `ValueOrToken::unwrap_token()` on a `Value` value"),
            Self::Token(t) => t,
        }
    }
}

#[derive(Default, Debug, Clone, Copy, Eq, PartialEq, Hash, get_size2::GetSize)]
pub enum Namespace<T> {
    #[default]
    Default,
    Explicit(T),
}

impl<T> Namespace<T> {
    pub fn from_value(value: T) -> Self
    where
        T: AsRef<str>,
    {
        if value.as_ref() == DEFAULT_NAMESPACE_NAME {
            Self::Default
        } else {
            Self::Explicit(value)
        }
    }

    pub fn from_option(value: Option<T>) -> Self
    where
        T: AsRef<str>,
    {
        value.map(Self::from_value).unwrap_or_else(|| Self::Default)
    }

    pub fn to_ref(&self) -> Namespace<&str>
    where
        T: AsRef<str>,
    {
        match self {
            Self::Default => Namespace::Default,
            Self::Explicit(ref value) => Namespace::Explicit(value.as_ref()),
        }
    }

    pub fn as_str(&self) -> &str
    where
        T: AsRef<str>,
    {
        match self {
            Self::Default => DEFAULT_NAMESPACE_NAME,
            Self::Explicit(ref value) => value.as_ref(),
        }
    }

    pub fn into_owned(self) -> Namespace<String>
    where
        T: Into<String>,
    {
        match self {
            Self::Default => Namespace::Default,
            Self::Explicit(value) => Namespace::Explicit(value.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PointReadValue {
    #[serde(rename = "value")]
    Value(UserMetadataValue),
    #[serde(rename = "missing")]
    Missing,
}

#[derive(Debug, Clone, Serialize)]
pub enum PointReadValueRef<'a> {
    #[serde(rename = "value")]
    Value(&'a UserMetadataValue),
    #[serde(rename = "missing")]
    Missing,
}

#[derive(Debug, Clone, Eq, PartialEq, get_size2::GetSize)]
pub enum AnyKey {
    PointRead(PointReadKey),
    Query(QueryKey),
}

impl AnyKey {
    pub fn to_view<'a>(&'a self) -> AnyKeyRef<'a> {
        match self {
            Self::PointRead(key) => AnyKeyRef::PointRead(key.to_ref()),
            Self::Query(key) => AnyKeyRef::Query(key.to_ref()),
        }
    }
}

impl std::hash::Hash for AnyKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.to_view().hash(state);
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub enum AnyKeyRef<'a> {
    PointRead(PointReadKeyRef<'a>),
    Query(QueryKeyRef<'a>),
}

impl AnyKeyRef<'_> {
    pub fn to_owned(self) -> AnyKey {
        match self {
            Self::PointRead(value) => AnyKey::PointRead(value.to_owned()),
            Self::Query(value) => AnyKey::Query(value.to_owned()),
        }
    }
}

impl Equivalent<AnyKey> for AnyKeyRef<'_> {
    fn equivalent(&self, key: &AnyKey) -> bool {
        self == &key.to_view()
    }
}

#[derive(Debug, Clone, Eq, PartialEq, get_size2::GetSize)]
pub struct PointReadKey {
    pub resource_uri: String,
    pub metadata_key: String,
    pub namespace: Namespace<String>,
}

impl PointReadKey {
    pub fn to_ref<'a>(&'a self) -> PointReadKeyRef<'a> {
        PointReadKeyRef {
            resource_uri: &self.resource_uri,
            metadata_key: &self.metadata_key,
            namespace: self.namespace.to_ref(),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, get_size2::GetSize)]
pub struct QueryKey {
    pub resource_uri: String,
    pub namespace: Namespace<String>,
}

impl QueryKey {
    pub fn to_ref<'a>(&'a self) -> QueryKeyRef<'a> {
        QueryKeyRef {
            resource_uri: &self.resource_uri,
            namespace: self.namespace.to_ref(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ResourceLocationRef<'a> {
    ResourceAddress(&'a str),
    ResourceIdentity(&'a str),
    FolderAddress(&'a str),
}

impl<'a> ResourceLocationRef<'a> {
    pub fn resource_uri(&self) -> &'a str {
        match self {
            Self::ResourceAddress(v) => v,
            Self::ResourceIdentity(v) => v,
            Self::FolderAddress(v) => v,
        }
    }

    pub fn to_owned(self) -> ResourceLocation {
        use storage_service_protos::nvidia::omniverse::storage::{
            filefolder::v1alpha::FolderAddress, fileobject::v1alpha::ResourceIdentity,
        };

        use crate::backend::metadata::resource_location::Resource;

        let resource = match self {
            Self::ResourceAddress(v) => Resource::ResourceAddress(v.to_string()),
            Self::ResourceIdentity(v) => Resource::ResourceIdentity(ResourceIdentity {
                encoded_identity: v.to_string(),
            }),
            Self::FolderAddress(v) => Resource::FolderAddress(FolderAddress { uri: v.to_string() }),
        };

        ResourceLocation {
            resource: Some(resource),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ExpandedPointReadKeyRef<'a> {
    pub resource_location: ResourceLocationRef<'a>,
    pub metadata_key: &'a str,
    pub namespace: Namespace<&'a str>,
}

impl<'a> ExpandedPointReadKeyRef<'a> {
    pub fn to_point_read_key_ref(self) -> PointReadKeyRef<'a> {
        PointReadKeyRef {
            resource_uri: self.resource_location.resource_uri(),
            metadata_key: self.metadata_key,
            namespace: self.namespace,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub struct PointReadKeyRef<'a> {
    pub resource_uri: &'a str,
    pub metadata_key: &'a str,
    pub namespace: Namespace<&'a str>,
}

impl PointReadKeyRef<'_> {
    pub fn to_owned(self) -> PointReadKey {
        PointReadKey {
            resource_uri: self.resource_uri.into(),
            metadata_key: self.metadata_key.into(),
            namespace: self.namespace.into_owned(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ExpandedQueryKeyRef<'a> {
    pub resource_location: ResourceLocationRef<'a>,
    pub namespace: Namespace<&'a str>,
}

impl<'a> ExpandedQueryKeyRef<'a> {
    pub fn to_query_key_ref(self) -> QueryKeyRef<'a> {
        QueryKeyRef {
            resource_uri: self.resource_location.resource_uri(),
            namespace: self.namespace,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub struct QueryKeyRef<'a> {
    pub resource_uri: &'a str,
    pub namespace: Namespace<&'a str>,
}

impl QueryKeyRef<'_> {
    pub fn to_owned(self) -> QueryKey {
        QueryKey {
            resource_uri: self.resource_uri.into(),
            namespace: self.namespace.into_owned(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Event {
    Reconnect,
    PointReadKeyInvalidate(PointReadKey),
    QueryKeyInvalidate(QueryKey),
}
