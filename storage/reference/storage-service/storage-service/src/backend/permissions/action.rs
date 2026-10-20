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

use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
};

use enum_map::{Enum, EnumMap};
use serde::{Deserialize, Serialize};

const ACTION_NAME_WRITE: &str = "write";
const ACTION_NAME_READ: &str = "read";

/// An enumeration of storage service actions.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Enum)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Stat,
    EnumerateVersions,
    Read,
    ReadFromAddress,
    Write,
    UploadPart,
    CompleteMultipartUpload,
    AbortMultipartUpload,
    CompleteRedirectUpload,
    CopyFrom,
    CopyTo,
    Delete,
    Enumerate,
    List,
    ListStat,
    FetchWriteTypeInfo,
    ListTopLevelAddresses,
    ListRoutes,
    GetMetadata,
    QueryMetadata,
    UpdateMetadata,
    DeleteMetadata,
    DeleteFolder,
    CreateFolder,
    GetFolderMode,
    MoveFrom,
    MoveTo,
}

impl Action {
    fn default_action_name(&self) -> &'static str {
        match self {
            Self::Stat
            | Self::EnumerateVersions
            | Self::Read
            | Self::ReadFromAddress
            | Self::CopyFrom
            | Self::Enumerate
            | Self::List
            | Self::ListStat
            | Self::FetchWriteTypeInfo
            | Self::ListTopLevelAddresses
            | Self::ListRoutes
            | Self::GetMetadata
            | Self::QueryMetadata
            | Self::GetFolderMode => ACTION_NAME_READ,
            Self::Write
            | Self::UploadPart
            | Self::CompleteMultipartUpload
            | Self::AbortMultipartUpload
            | Self::CompleteRedirectUpload
            | Self::CopyTo
            | Self::Delete
            | Self::UpdateMetadata
            | Self::DeleteMetadata
            | Self::CreateFolder
            | Self::DeleteFolder
            | Self::MoveFrom
            | Self::MoveTo => ACTION_NAME_WRITE,
        }
    }
}

/// A map of actions to their corresponding names.
#[derive(Debug, Clone)]
pub struct ActionMap {
    inner: EnumMap<Action, Cow<'static, str>>,
    unique_action_names: Vec<Cow<'static, str>>,
}

impl ActionMap {
    /// Return the name for the specified action.
    pub fn get_action_name(&self, action: Action) -> &str {
        self.inner[action].as_ref()
    }

    /// Return the list of unique action names in this map.
    pub fn unique_action_names(&self) -> &[Cow<'static, str>] {
        &self.unique_action_names
    }

    /// Iterate over all unique action names in this map.
    pub fn iter_unique_actions(&self) -> impl Iterator<Item = &str> {
        self.unique_action_names.iter().map(AsRef::as_ref)
    }
}

impl Default for ActionMap {
    fn default() -> Self {
        Self {
            inner: EnumMap::from_fn(|key: Action| Cow::Borrowed(key.default_action_name())),
            unique_action_names: vec![ACTION_NAME_READ.into(), ACTION_NAME_WRITE.into()],
        }
    }
}

impl<'de> Deserialize<'de> for ActionMap {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let mut deserialized = HashMap::<Action, String>::deserialize(deserializer)?;
        let inner = EnumMap::from_fn(|key| {
            deserialized
                .remove(&key)
                .map(Cow::Owned)
                .unwrap_or_else(|| Cow::Borrowed(key.default_action_name()))
        });
        let unique_action_names: HashSet<_> = inner.values().collect();

        Ok(Self {
            unique_action_names: unique_action_names.into_iter().cloned().collect(),
            inner,
        })
    }
}
