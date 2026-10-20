// SPDX-FileCopyrightText: Copyright 2026 NVIDIA CORPORATION & AFFILIATES
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

use serde::Deserialize;

use crate::models::{Group, User};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphUser {
    pub id: String,
    pub display_name: Option<String>,
    pub given_name: Option<String>,
    pub surname: Option<String>,
    pub mail: Option<String>,
    pub user_principal_name: Option<String>,
    pub account_enabled: Option<bool>,
    pub job_title: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphGroup {
    pub id: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GraphListResponse<T> {
    pub value: Vec<T>,
    #[serde(rename = "@odata.nextLink")]
    pub next_link: Option<String>,
}

/// A single `appRoleAssignedTo` entry for an enterprise application, describing
/// one principal (user, group, or service principal) assigned to the app.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphAppRoleAssignment {
    pub principal_id: String,
    pub principal_display_name: Option<String>,
    pub principal_type: Option<String>,
}

/// Minimal service principal projection used to resolve an application's own
/// service principal object id from its `appId`.
#[derive(Debug, Clone, Deserialize)]
pub struct GraphServicePrincipalRef {
    pub id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GraphRemoved {
    pub reason: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GraphDeltaItem<T> {
    #[serde(flatten)]
    pub inner: T,
    #[serde(rename = "@removed")]
    pub removed: Option<GraphRemoved>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GraphDeltaResponse<T> {
    pub value: Vec<GraphDeltaItem<T>>,
    #[serde(rename = "@odata.nextLink")]
    pub next_link: Option<String>,
    #[serde(rename = "@odata.deltaLink")]
    pub delta_link: Option<String>,
}

/// A single member entry inside a `members@delta` array.
#[derive(Debug, Clone, Deserialize)]
pub struct GraphMemberRef {
    pub id: String,
    #[serde(rename = "@removed")]
    pub removed: Option<GraphRemoved>,
}

/// One item in a groups delta response, carrying both property changes and
/// direct-membership additions/removals via the `members@delta` expansion.
#[derive(Debug, Clone, Deserialize)]
pub struct GraphGroupDeltaItem {
    #[serde(flatten)]
    pub inner: GraphGroup,
    #[serde(rename = "@removed")]
    pub removed: Option<GraphRemoved>,
    #[serde(rename = "members@delta", default)]
    pub members_delta: Vec<GraphMemberRef>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GraphGroupDeltaResponse {
    pub value: Vec<GraphGroupDeltaItem>,
    #[serde(rename = "@odata.nextLink")]
    pub next_link: Option<String>,
    #[serde(rename = "@odata.deltaLink")]
    pub delta_link: Option<String>,
}

impl From<GraphUser> for User {
    fn from(g: GraphUser) -> Self {
        Self {
            id: g.id,
            display_name: g.display_name.unwrap_or_default(),
            given_name: g.given_name,
            family_name: g.surname,
            email: g.mail,
            username: g.user_principal_name.unwrap_or_default(),
            active: g.account_enabled,
            job_title: g.job_title,
        }
    }
}

impl From<GraphGroup> for Group {
    fn from(g: GraphGroup) -> Self {
        Self {
            id: g.id,
            display_name: g.display_name.unwrap_or_default(),
            description: g.description,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_user_converts_to_user() {
        let graph = GraphUser {
            id: "uid-1".into(),
            display_name: Some("Jane Doe".into()),
            given_name: Some("Jane".into()),
            surname: Some("Doe".into()),
            mail: Some("jane@example.com".into()),
            user_principal_name: Some("jane@corp.com".into()),
            account_enabled: Some(true),
            job_title: Some("Engineer".into()),
        };
        let user: User = graph.into();
        assert_eq!(user.id, "uid-1");
        assert_eq!(user.display_name, "Jane Doe");
        assert_eq!(user.family_name.as_deref(), Some("Doe"));
        assert_eq!(user.email.as_deref(), Some("jane@example.com"));
        assert_eq!(user.username, "jane@corp.com");
        assert_eq!(user.active, Some(true));
    }

    #[test]
    fn graph_group_converts_to_group() {
        let graph = GraphGroup {
            id: "gid-1".into(),
            display_name: Some("Admins".into()),
            description: Some("Admin group".into()),
        };
        let group: Group = graph.into();
        assert_eq!(group.id, "gid-1");
        assert_eq!(group.display_name, "Admins");
        assert_eq!(group.description.as_deref(), Some("Admin group"));
    }
}
