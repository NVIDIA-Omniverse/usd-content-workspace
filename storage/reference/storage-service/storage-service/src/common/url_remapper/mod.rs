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

pub mod error;

use either::Either;
use indexmap::IndexMap;
use percent_encoding::percent_decode;
use serde::{Deserialize, Serialize};
use std::{borrow::Cow, collections::HashMap, fmt::Debug, mem::take, ops::Index, slice::SliceIndex, sync::Arc};
use storage_core::traits::{
    capabilities::{v1alpha, v1beta},
    Request,
};
use tracing::{debug, warn};

use storage_service_protos::google::protobuf::Value;

use crate::{
    backend::metadata::{GetMetadataBatchRequestEntry, GetMetadataBatchResponseEntry, Metadata},
    common::{
        error::PrettyPrintErrorExt,
        iterator::TryCollectExt,
        url::{get_etag_query_parameter, parse_checkpoint_number, split_query_from_url},
        url_remapper::error::{CheckpointMetadataParseError, ConfigError, UrlParseError, UrlRewriteError},
    },
};

const URL_SCHEME_OMNIVERSE: &str = "omniverse";

/// A URL rewrite result.
pub enum RewriteResult<T = String> {
    Unchanged,
    Mapped(T, RuleTag),
}

impl<T> RewriteResult<T> {
    pub fn map<R>(self, map: impl FnOnce(T) -> R) -> RewriteResult<R> {
        match self {
            Self::Mapped(value, tag) => RewriteResult::Mapped(map(value), tag),
            Self::Unchanged => RewriteResult::Unchanged,
        }
    }

    pub fn unwrap_or_else(self, default: impl FnOnce() -> T) -> T {
        match self {
            Self::Mapped(value, _) => value,
            Self::Unchanged => default(),
        }
    }

    pub fn unwrap_or(self, value: T) -> T {
        self.unwrap_or_else(|| value)
    }
}

impl<T: Debug> Debug for RewriteResult<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unchanged => f.debug_tuple("Unchanged").finish(),
            Self::Mapped(value, rule_tag) => f.debug_tuple("Mapped").field(value).field(rule_tag).finish(),
        }
    }
}

/// An enumeration of url rewrite modes.
#[derive(Debug, Clone, Copy)]
pub enum RewriteMode {
    /// Rewrite a url fuzzy. Path segments differing in the trailing slash only will be considered identical.
    /// The Fuzzy rewrite mode will successfully rewrite:
    ///     scheme://mapped-host/mapped-path -> https://original-host/original-path/
    /// for the rule:
    ///     sceheme://mapped-host/mapped-path/ -> https://original-host/original-path/
    /// despite the trailing slash not being explicitly set in the mapped url.
    Fuzzy,
    /// Rewrite a url precisely. Path segments with and without the trailing slash will be cosndiered different.
    Precise,
}

/// A rewrite rule tag;
#[derive(Debug, Clone, Copy)]
pub struct RuleTag(usize);

/// A url prefix rewrite rule.
#[derive(Debug, Clone)]
pub struct RewriteRule {
    url_prefix_from: UrlPrefixComponents,
    url_prefix_to: UrlPrefixComponents,
    supports_checkpoint_mapping: bool,
    list_in_top_level_addresses: bool,
    /// The original folder address `url_prefix_to` rewrites to, precomputed at construction.
    target_folder_address: String,
}

impl RewriteRule {
    /// Get a top leve address entry for this rule.
    pub fn get_top_level_address_entry_v1alpha(&self) -> v1alpha::TopLevelAddressEntry {
        debug_assert!(self.url_prefix_from.encoded_path.ends_with("/"));
        debug_assert!(self.url_prefix_from.encoded_path.starts_with("/"));
        debug_assert!(self.url_prefix_from.decoded_path.ends_with(b"/"));
        debug_assert!(self.url_prefix_from.decoded_path.starts_with(b"/"));

        v1alpha::TopLevelAddressEntry {
            top_level_address: format!(
                "{}://{}{}",
                self.url_prefix_from.scheme, self.url_prefix_from.authority, self.url_prefix_from.encoded_path
            ),
        }
    }

    /// Get a top leve address entry for this rule.
    pub fn get_top_level_address_entry_v1beta(&self) -> v1beta::TopLevelAddressEntry {
        debug_assert!(self.url_prefix_from.encoded_path.ends_with("/"));
        debug_assert!(self.url_prefix_from.encoded_path.starts_with("/"));
        debug_assert!(self.url_prefix_from.decoded_path.ends_with(b"/"));
        debug_assert!(self.url_prefix_from.decoded_path.starts_with(b"/"));

        v1beta::TopLevelAddressEntry {
            top_level_address: format!(
                "{}://{}{}",
                self.url_prefix_from.scheme, self.url_prefix_from.authority, self.url_prefix_from.encoded_path
            ),
        }
    }

    /// Get the original folder address this rule rewrites to.
    pub fn get_target_folder_address(&self) -> &str {
        &self.target_folder_address
    }

    /// Get a route for this rule.
    pub fn get_route_v1alpha(&self) -> v1alpha::Route {
        use v1alpha::{route::RouteType, Route};

        debug_assert!(self.url_prefix_from.encoded_path.ends_with("/"));
        debug_assert!(self.url_prefix_from.encoded_path.starts_with("/"));
        debug_assert!(self.url_prefix_from.decoded_path.ends_with(b"/"));
        debug_assert!(self.url_prefix_from.decoded_path.starts_with(b"/"));

        Route {
            route_type: Some(RouteType::WildcardPattern(format!(
                "{}://{}{}**",
                self.url_prefix_from.scheme, self.url_prefix_from.authority, self.url_prefix_from.encoded_path
            ))),
        }
    }
}

/// A checkpoint metadata entry.
#[derive(Debug, Clone)]
pub struct CheckpointMetadata {
    pub id: u64,
    pub resource_identity: String,
}

impl CheckpointMetadata {
    fn parse_one(idx: usize, value: Value) -> Result<Self, CheckpointMetadataParseError> {
        use storage_service_protos::google::protobuf::value::Kind;

        let Some(Kind::StructValue(mut value)) = value.kind else {
            return Err(CheckpointMetadataParseError::StructExpected(idx));
        };

        let id = {
            let Some(id_value) = value.fields.get("id") else {
                return Err(CheckpointMetadataParseError::IdMissing(idx));
            };

            let Some(Kind::NumberValue(id)) = id_value.kind.as_ref() else {
                return Err(CheckpointMetadataParseError::IdInvalid(idx));
            };

            *id
        };

        let resource_identity = {
            let Some(resource_identity_value) = value.fields.get_mut("resource_identity") else {
                return Err(CheckpointMetadataParseError::ResourceIdentityMissing(idx));
            };

            let Some(Kind::StringValue(resource_identity)) = resource_identity_value.kind.as_mut() else {
                return Err(CheckpointMetadataParseError::ResourceIdentityInvalid(idx));
            };

            take(resource_identity)
        };

        Ok(Self {
            id: id as u64,
            resource_identity,
        })
    }

    fn parse_all(value: Value) -> Result<Vec<Self>, CheckpointMetadataParseError> {
        use storage_service_protos::google::protobuf::value::Kind;

        let Some(Kind::ListValue(value)) = value.kind else {
            return Err(CheckpointMetadataParseError::RootListExpected);
        };

        value
            .values
            .into_iter()
            .enumerate()
            .map(|(idx, value)| Self::parse_one(idx, value))
            .try_collect()
    }
}

/// Checkpoint metadata cache.
#[derive(Default)]
pub struct CheckpointMetadataCache {
    last_resource_address: Option<String>,
    last_metadata_entries: Vec<CheckpointMetadata>,
}

/// URL remapper configuration.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct UrlRemapperConfig {
    pub enabled: bool,
    pub rewrite_rules: HashMap<String, RewriteRuleConfig>,
    pub checkpoint_metadata_key: String,
    pub expose_child_rules_as_folders: bool,
}

/// The configuration of a single rewrite rule.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct RewriteRuleConfig {
    /// The original-URL prefix the mapped prefix translates to.
    pub target: String,
    /// Whether this alias is listed in `ListTopLevelAddresses`. Defaults to true.
    /// Aliases are always listed in `ListRoutes`.
    #[serde(default = "crate::config::default_true")]
    pub list_in_top_level_addresses: bool,
}

impl Default for UrlRemapperConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            rewrite_rules: Default::default(),
            checkpoint_metadata_key: "checkpoints".into(),
            expose_child_rules_as_folders: false,
        }
    }
}

/// URL mapper.
pub struct UrlRemapper<T: Metadata> {
    pub(super) inner: Arc<UrlRemapperInner<T>>,
}

impl<T: Metadata> UrlRemapper<T> {
    /// Create a url mapper given a set of rewrite rules and checkpoint metadta key.
    ///
    /// `endpoint_roots` pairs each physical backend endpoint URL (e.g.
    /// `https://account.blob.core.windows.net/`) with its [`RootListing`], which says whether the
    /// endpoint's own root is itself a browsable top-level address. A rewrite rule nested under a
    /// root is treated as a mount hosted by that endpoint, so it does not need a covering alias to be
    /// a valid, reachable mount.
    pub fn new<S: AsRef<str>>(
        metadata: Option<T>,
        config: UrlRemapperConfig,
        endpoint_roots: impl IntoIterator<Item = (S, RootListing)>,
    ) -> Result<Option<Self>, ConfigError> {
        if !config.enabled {
            return Ok(None);
        }

        // The single source of truth for what each backend endpoint hosts; both endpoint-aware
        // behaviors (hierarchy validation and top-level suppression) flow from it. See
        // [`EndpointRoots`].
        let endpoint_roots = EndpointRoots::from_config(endpoint_roots);

        let mut rewrite_rules: Vec<RewriteRule> = config
            .rewrite_rules
            .into_iter()
            .map(|(url_prefix_from, rule_config)| {
                let url_prefix_from =
                    UrlPrefixComponents::parse(&url_prefix_from).map_err(ConfigError::SourcePrefixParseFailed)?;
                let url_prefix_to =
                    UrlPrefixComponents::parse(&rule_config.target).map_err(ConfigError::TargetPrefixParseFailed)?;
                Ok(RewriteRule {
                    supports_checkpoint_mapping: url_prefix_from.scheme == URL_SCHEME_OMNIVERSE,
                    target_folder_address: url_prefix_to.format_with_suffix("", None, None),
                    list_in_top_level_addresses: rule_config.list_in_top_level_addresses,
                    url_prefix_from,
                    url_prefix_to,
                })
            })
            .try_collect()?;

        rewrite_rules.sort_unstable_by(|a, b| {
            a.url_prefix_from
                .decoded_path
                .len()
                .cmp(&b.url_prefix_from.decoded_path.len())
                .reverse()
        });

        if config.expose_child_rules_as_folders {
            // Validation uses every endpoint root: a mount under a hidden endpoint is still
            // structurally hosted by it (just not advertised).
            validate_rule_hierarchy(&rewrite_rules, &endpoint_roots)?;
        }

        let synthetic_index = if config.expose_child_rules_as_folders {
            SyntheticChildrenIndex::build(&rewrite_rules)
        } else {
            SyntheticChildrenIndex::default()
        };

        Ok(Some(Self {
            inner: Arc::new(UrlRemapperInner {
                metadata,
                rewrite_rules,
                checkpoint_metadata_key: config.checkpoint_metadata_key,
                expose_child_rules_as_folders: config.expose_child_rules_as_folders,
                synthetic_index,
                endpoint_roots,
            }),
        }))
    }

    /// Return synthetic direct-child folder URLs of `parent_url` derived
    /// from the configured rewrite rules.
    ///
    /// The returned URLs are in the *user* namespace (the same namespace
    /// as `parent_url`) and end with `/`. Each rule whose source URL is a
    /// strict descendant of `parent_url` contributes exactly one synthetic
    /// folder named for the first path segment between `parent_url` and
    /// the rule source -- so a parent listed at `proto://host/parent/`
    /// surfaces `proto://host/parent/Folder/` for *any* rule whose source
    /// starts with `proto://host/parent/Folder/`, regardless of how many
    /// path segments are nested below it (multi-level mount trees collapse
    /// to the direct-child segment).
    ///
    /// Returns an empty vector when the feature is disabled, when
    /// `parent_url` does not parse, or when no rule sits below it.
    pub fn synthetic_children_of(&self, parent_url: &str) -> Vec<String> {
        if !self.inner.expose_child_rules_as_folders {
            return Vec::new();
        }
        self.inner.synthetic_children_of(parent_url)
    }

    /// Return `true` when `url` exactly matches a rewrite-rule source.
    ///
    /// Used by the filefolder decorator to reject `delete_folder` /
    /// `create_folder` requests targeting a mount-point itself -- those
    /// operations would otherwise rewrite the URL to the mounted target
    /// and silently delete / create against another bucket. Match is
    /// fuzzy on the trailing slash so callers can pass either `mount/`
    /// or `mount`.
    pub fn is_mount_point(&self, url: &str) -> bool {
        self.inner.is_mount_point(url)
    }

    /// Iterate over rewrite rules.
    pub fn rewrite_rules(&self) -> impl Iterator<Item = &RewriteRule> {
        self.inner.rewrite_rules.iter()
    }

    /// The rewrite rules surfaced as top-level addresses. Rules that opted out of
    /// `ListTopLevelAddresses` are excluded. With `expose_child_rules_as_folders` a
    /// nested rule is reachable via its parent's synthetic folder, so only rules with no parent
    /// -- neither an ancestor rule nor a *self-listed* configured endpoint -- are surfaced;
    /// otherwise nested mounts would be undiscoverable, and endpoint-hosted mounts would double as
    /// browsable folders and standalone top-level entries. A mount whose endpoint root is not itself
    /// a browsable top-level address (a hidden endpoint, or an Azure account root the adapter expands
    /// into containers) is not reachable that way, so it is not suppressed.
    pub fn top_level_rewrite_rules(&self) -> impl Iterator<Item = &RewriteRule> {
        let expose = self.inner.expose_child_rules_as_folders;
        let rules = self.inner.rewrite_rules.as_slice();
        let endpoint_roots = &self.inner.endpoint_roots;
        rules.iter().filter(move |rule| {
            rule.list_in_top_level_addresses
                && (!expose || (!has_ancestor_rule(rule, rules) && !endpoint_roots.self_listed_hosts(rule)))
        })
    }

    /// Rewrite an unversioned request URL.
    pub fn fwd_rewrite_unversioned_url(&self, url: &str, mode: RewriteMode) -> RewriteResult {
        self.inner.fwd_rewrite_unversioned_url(url, mode)
    }

    pub fn fwd_rewrite_folder_url(&self, url: &str) -> RewriteResult {
        self.inner.fwd_rewrite_unversioned_url(url, RewriteMode::Fuzzy)
    }

    /// Rewrite a versioned request URL.
    pub async fn fwd_rewrite_versioned_url(
        &self,
        url: &str,
        mode: RewriteMode,
    ) -> Result<RewriteResult, UrlRewriteError> {
        self.inner.fwd_rewrite_versioned_url(url, mode).await
    }

    /// Rewrite an unversioned resource URL.
    pub fn rev_rewrite_unversioned_url(&self, url: &str, mode: RewriteMode) -> Option<String> {
        self.inner.rev_rewrite_unversioned_url(None, url, mode)
    }

    /// Rewrite an unversioned resource URL using the specified rule.
    pub fn rev_rewrite_unversioned_url_with_tag(&self, tag: RuleTag, url: &str, mode: RewriteMode) -> Option<String> {
        self.inner.rev_rewrite_unversioned_url(Some(tag), url, mode)
    }

    /// Rewrite a versioned resource URL using the specified rule.
    pub async fn rev_rewrite_versioned_url_with_tag(
        &self,
        tag: RuleTag,
        url: &str,
        cache: Option<&mut CheckpointMetadataCache>,
    ) -> Result<Option<(String, bool)>, UrlRewriteError> {
        self.inner.rev_rewrite_versioned_url_with_tag(tag, url, cache).await
    }

    /// Rewrite versioned resource URLs in a batch using the specified rule.
    pub async fn rev_rewrite_versioned_url_batch_with_tag(
        &self,
        tag: RuleTag,
        urls: impl IntoIterator<Item = String>,
    ) -> Result<Vec<String>, UrlRewriteError> {
        self.inner.rev_rewrite_versioned_url_batch_with_tag(tag, urls).await
    }
}

impl<T: Metadata> Clone for UrlRemapper<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T: Debug + Metadata> Debug for UrlRemapper<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UrlRemapper").field("inner", &self.inner).finish()
    }
}

pub struct UrlRemapperInner<T> {
    pub(super) metadata: Option<T>,
    pub(super) rewrite_rules: Vec<RewriteRule>,
    pub(super) checkpoint_metadata_key: String,
    pub(super) expose_child_rules_as_folders: bool,
    pub(super) synthetic_index: SyntheticChildrenIndex,
    // The backend endpoint roots and what each one hosts; the single source of truth for the
    // remapper's endpoint-aware behavior. See [`EndpointRoots`].
    endpoint_roots: EndpointRoots,
}

#[derive(Debug, Default)]
pub struct SyntheticChildrenIndex {
    by_namespace: IndexMap<(String, String), SyntheticChildrenNode>,
}

#[derive(Debug, Default)]
struct SyntheticChildrenNode {
    children: IndexMap<Vec<u8>, SyntheticChildrenChild>,
}

impl SyntheticChildrenNode {
    fn sort_all(&mut self) {
        self.children.sort_keys();
        for child in self.children.values_mut() {
            child.node.sort_all();
        }
    }
}

#[derive(Debug)]
struct SyntheticChildrenChild {
    /// Full synthetic URL ending with `/`, in the encoded form taken
    /// from the original rule source.
    synthetic_url: String,
    node: SyntheticChildrenNode,
}

impl SyntheticChildrenIndex {
    fn build(rules: &[RewriteRule]) -> Self {
        let mut index = Self::default();
        for rule in rules {
            let src = &rule.url_prefix_from;
            debug_assert!(src.encoded_path.starts_with('/'));
            debug_assert!(src.encoded_path.ends_with('/'));
            debug_assert!(src.decoded_path.starts_with(b"/"));
            debug_assert!(src.decoded_path.ends_with(b"/"));
            let inner_encoded = strip_one_leading_and_trailing_slash(src.encoded_path.as_bytes());
            // SAFETY: `inner_encoded` is a sub-slice of `src.encoded_path` (UTF-8)
            // with at most one ASCII slash stripped from each end; still UTF-8.
            let inner_encoded = unsafe { std::str::from_utf8_unchecked(inner_encoded) };
            if inner_encoded.is_empty() {
                continue;
            }
            let decoded_segments = decoded_path_segments(&src.decoded_path);
            let mut node = index
                .by_namespace
                .entry((src.scheme.clone(), src.authority.clone()))
                .or_default();
            let mut url_so_far = format!("{}://{}/", src.scheme, src.authority);
            for (encoded_segment, decoded_segment) in inner_encoded.split('/').zip(decoded_segments) {
                url_so_far.push_str(encoded_segment);
                url_so_far.push('/');
                let child = node
                    .children
                    .entry(decoded_segment.to_vec() as Vec<u8>)
                    .or_insert_with(|| SyntheticChildrenChild {
                        synthetic_url: url_so_far.clone(),
                        node: SyntheticChildrenNode::default(),
                    });
                node = &mut child.node;
            }
        }
        index.sort_all();
        index
    }

    fn sort_all(&mut self) {
        self.by_namespace.sort_keys();
        for node in self.by_namespace.values_mut() {
            node.sort_all();
        }
    }

    fn synthetic_children_of(&self, parent_url: &str) -> Vec<String> {
        let Ok(parent) = UrlComponents::parse(parent_url) else {
            return Vec::new();
        };
        let parent_decoded: Vec<u8> = percent_decode(parent.path().as_bytes()).collect();

        let Some(mut node) = self
            .by_namespace
            .get(&(parent.scheme().to_owned(), parent.authority().to_owned()))
        else {
            return Vec::new();
        };

        for segment in decoded_path_segments(&parent_decoded) {
            let Some(child) = node.children.get(segment) else {
                return Vec::new();
            };
            node = &child.node;
        }

        node.children
            .values()
            .map(|child| child.synthetic_url.clone())
            .collect()
    }
}

fn decoded_path_segments(decoded_path: &[u8]) -> impl Iterator<Item = &[u8]> + '_ {
    let without_leading = decoded_path.strip_prefix(b"/").unwrap_or(decoded_path);
    if without_leading.is_empty() {
        return Either::Left(std::iter::empty());
    }
    let inner = without_leading.strip_suffix(b"/").unwrap_or(without_leading);
    Either::Right(inner.split(|&b| b == b'/'))
}

/// Strip at most one `/` from the start and one from the end of `bytes`.
/// Unlike `trim_start_matches`/`trim_end_matches`, multi-slash runs are
/// not collapsed -- only the single bracketing slash that
/// `UrlPrefixComponents::parse` mandates is removed.
fn strip_one_leading_and_trailing_slash(bytes: &[u8]) -> &[u8] {
    let bytes = bytes.strip_prefix(b"/").unwrap_or(bytes);
    bytes.strip_suffix(b"/").unwrap_or(bytes)
}

impl<T: Metadata> UrlRemapperInner<T> {
    /// Rewrite an unversioned resource URL.
    fn fwd_rewrite_unversioned_url(&self, url: &str, mode: RewriteMode) -> RewriteResult {
        let Ok(components) = UrlComponents::parse(url).inspect_invalid_request_url() else {
            return RewriteResult::Unchanged;
        };

        debug!(?url, ?self.rewrite_rules);

        for (rule_id, rule) in self.rewrite_rules.iter().enumerate() {
            if let Some(path_suffix) =
                rule.url_prefix_from
                    .strip_from(components.scheme(), components.authority(), components.path(), mode)
            {
                debug!(
                    ?url,
                    ?path_suffix,
                    ?rule,
                    ?rule_id,
                    "selected a rewrite rule for request mapping"
                );

                return RewriteResult::Mapped(
                    rule.url_prefix_to
                        .format_with_suffix(path_suffix, components.query(), components.fragment()),
                    RuleTag(rule_id),
                );
            }
        }

        debug!(?url, ?mode, "no rewrite rule found for the url");

        RewriteResult::Unchanged
    }

    /// Rewrite a versioned resource URL.
    async fn fwd_rewrite_versioned_url(&self, url: &str, mode: RewriteMode) -> Result<RewriteResult, UrlRewriteError> {
        // We assume that resource addresses and identities are URLs, so that to get a resource address for an
        // identity one shall simply strip the query parameters from the latter.
        let Ok(components) = UrlComponents::parse(url).inspect_invalid_request_url() else {
            return Ok(RewriteResult::Unchanged);
        };

        for (rule_id, rule) in self.rewrite_rules.iter().enumerate() {
            let Some(path_suffix) =
                rule.url_prefix_from
                    .strip_from(components.scheme(), components.authority(), components.path(), mode)
            else {
                continue;
            };

            debug!(
                ?url,
                ?path_suffix,
                ?rule,
                ?rule_id,
                "selected a rewrite rule for request remapping"
            );

            let simple_rewrite_result = || {
                RewriteResult::Mapped(
                    rule.url_prefix_to
                        .format_with_suffix(path_suffix, components.query(), components.fragment()),
                    RuleTag(rule_id),
                )
            };

            // Rewrite the checkpoint number if the URL is an omniverse URL.
            if !rule.supports_checkpoint_mapping {
                debug!("rewrite rule does not support checkpoints");

                return Ok(simple_rewrite_result());
            }

            // If there's no query or the checkpoint number is missing from the query string, replace the prefix only.
            let Some(query) = components.query() else {
                debug!("rewrite rule supports checkpoints but the query is missing");

                return Ok(simple_rewrite_result());
            };

            let Some(checkpoint) = parse_checkpoint_number(query) else {
                debug!("rewrite rule supports checkpoints but the query lacks the checkpoint number");

                return Ok(simple_rewrite_result());
            };

            // Otherwise, attempt to find a piece of metadata for the checkpoint and error if it's missing.
            let original_resource_address = rule.url_prefix_to.format_with_suffix(path_suffix, None, None);
            let metadata_entries = self.fetch_checkpoint_metadata(original_resource_address).await?;
            for entry in metadata_entries.into_iter() {
                if entry.id == checkpoint {
                    debug!(?entry, "found a checkpoint metadata entry");

                    return Ok(RewriteResult::Mapped(entry.resource_identity, RuleTag(rule_id)));
                }
            }

            return Err(UrlRewriteError::missing_checkpoint_metadata(checkpoint));
        }

        debug!(?url, ?mode, "no rewrite rule found for the url");

        Ok(RewriteResult::Unchanged)
    }

    /// Reverse-rewrite an unversioned url.
    fn rev_rewrite_unversioned_url(&self, rule_tag: Option<RuleTag>, url: &str, mode: RewriteMode) -> Option<String> {
        if let Some(rule_tag) = rule_tag {
            debug_assert!(self.rewrite_rules.len() > rule_tag.0);

            let rule = &self.rewrite_rules[rule_tag.0];
            self.rev_rewrite_unversioned_url_with_rules([rule], url, mode)
        } else {
            self.rev_rewrite_unversioned_url_with_rules(self.rewrite_rules.iter(), url, mode)
        }
    }

    /// Reverse-rewrite an unversioned url using one or more selected rules.
    fn rev_rewrite_unversioned_url_with_rules<'a>(
        &'a self,
        rules: impl IntoIterator<Item = &'a RewriteRule>,
        url: &str,
        mode: RewriteMode,
    ) -> Option<String> {
        let Ok(components) = UrlComponents::parse(url).inspect_invalid_response_url() else {
            return None;
        };

        for rule in rules.into_iter() {
            let Some(path_suffix) =
                rule.url_prefix_to
                    .strip_from(components.scheme(), components.authority(), components.path(), mode)
            else {
                continue;
            };

            return Some(rule.url_prefix_from.format_with_suffix(
                path_suffix,
                components.query(),
                components.fragment(),
            ));
        }

        debug!(?url, ?mode, "no rewrite rule found");

        None
    }

    /// Reverse-rewrite a versioned url.
    async fn rev_rewrite_versioned_url_with_tag(
        &self,
        rule_tag: RuleTag,
        url: &str,
        cache: Option<&mut CheckpointMetadataCache>,
    ) -> Result<Option<(String, bool)>, UrlRewriteError> {
        debug_assert!(self.rewrite_rules.len() > rule_tag.0);

        let Ok(components) = UrlComponents::parse(url).inspect_invalid_response_url() else {
            return Ok(None);
        };

        let rule = &self.rewrite_rules[rule_tag.0];
        let maybe_original_etag = components.query().map(get_etag_query_parameter).unwrap_or_default();
        let Some(path_suffix) = rule.url_prefix_to.strip_from(
            components.scheme(),
            components.authority(),
            components.path(),
            RewriteMode::Precise,
        ) else {
            return Ok(None);
        };

        // Proceed to the checkpoint remapping if the source URL is an Omniverse URL, and if its query parameters
        // contain the etag value that we can use for mapping between identities and checkpoint numbers.
        let (true, Some(original_query), Some(original_etag)) = (
            rule.supports_checkpoint_mapping,
            components.query(),
            maybe_original_etag,
        ) else {
            return Ok(Some((
                rule.url_prefix_from
                    .format_with_suffix(path_suffix, components.query(), components.fragment()),
                false,
            )));
        };

        let Some(checkpoint) = self
            .resolve_resorce_identity_to_checkpoint_id(components.resource_url(), original_etag, cache)
            .await?
        else {
            return Ok(Some((
                rule.url_prefix_from
                    .format_with_suffix(path_suffix, Some(original_query), components.fragment()),
                false,
            )));
        };

        Ok(Some((
            rule.url_prefix_from
                .format_with_checkpoint_suffix(path_suffix, checkpoint),
            true,
        )))
    }

    /// Reverse-rewrite a batch of versioned URLs.
    async fn rev_rewrite_versioned_url_batch_with_tag(
        &self,
        rule_tag: RuleTag,
        urls: impl IntoIterator<Item = String>,
    ) -> Result<Vec<String>, UrlRewriteError> {
        use crate::backend::metadata::GetMetadataBatchRequest;

        debug_assert!(self.rewrite_rules.len() > rule_tag.0);

        // For each url, we parse its url components and check whether an etag query parameter is in the query string.
        let rule = &self.rewrite_rules[rule_tag.0];
        let mut components_or_urls: Vec<_> = urls
            .into_iter()
            .map(|url| match UrlComponents::parse(url).inspect_invalid_response_url() {
                Ok(url_components) => {
                    if rule.supports_checkpoint_mapping {
                        if url_components.query().and_then(get_etag_query_parameter).is_some() {
                            Either::Left((url_components, true))
                        } else {
                            Either::Left((url_components, false))
                        }
                    } else {
                        Either::Left((url_components, false))
                    }
                },
                Err((url, _)) => Either::Right(url),
            })
            .collect();

        // Query metadata for the urls that contain the etag query parameter. We don't want to query metadata if the
        // backend is not configured or if the rule does not support checkpoint mapping.
        let metadata = if let (true, Some(metadata)) = (rule.supports_checkpoint_mapping, self.metadata.as_ref()) {
            let requests: Vec<_> = components_or_urls
                .iter()
                .filter_map(|components_or_url| {
                    if let Either::Left((components, true)) = components_or_url.as_ref() {
                        Some(self.create_batch_get_request_entry(components.resource_url()))
                    } else {
                        None
                    }
                })
                .collect();

            // Don't make a request if we have nothing to request.
            if !requests.is_empty() {
                let requests_len = requests.len();
                let mut results = metadata
                    .get_metadata_batch(Request::new(GetMetadataBatchRequest { requests }))
                    .await
                    .map_err(UrlRewriteError::fetch_failed)?
                    .results;

                // Ordering of metadata entries is not guaranteed in the response.
                results.sort_by_key(|a| a.request_index);

                debug_assert_eq!(results.len(), requests_len);
                debug_assert!(results.iter().enumerate().all(|(idx, e)| e.request_index == idx as u32));

                results
            } else {
                Default::default()
            }
        } else {
            Default::default()
        };

        // Reverse-rewrite resource urls.
        let mut metadata = metadata.into_iter();
        for components_or_url in components_or_urls.iter_mut() {
            let Either::Left((components, rewrite_query)) = components_or_url.as_mut() else {
                continue;
            };

            let maybe_checkpoint_metadata_value = if *rewrite_query {
                if let Some(entry) = metadata.next() {
                    get_user_metadata_value(entry)?
                } else {
                    None
                }
            } else {
                None
            };

            let Some(path_suffix) = rule.url_prefix_to.strip_from(
                components.scheme(),
                components.authority(),
                components.path(),
                RewriteMode::Precise,
            ) else {
                continue;
            };

            let original_query = components.query();
            let maybe_original_etag = if *rewrite_query {
                original_query.and_then(get_etag_query_parameter)
            } else {
                None
            };

            // If the etag value is missing from the original query or the checkpoint metadata could not be
            // found for the entry, replace the url prefix only.
            let (Some(original_query), Some(original_etag), Some(checkpoint_metadata_value)) =
                (original_query, maybe_original_etag, maybe_checkpoint_metadata_value)
            else {
                *components_or_url = Either::Right(rule.url_prefix_from.format_with_suffix(
                    path_suffix,
                    original_query,
                    components.fragment(),
                ));
                continue;
            };

            // Try to find the checkpoint number by the etag value.
            let checkpoint_metadata = CheckpointMetadata::parse_all(checkpoint_metadata_value)
                .map_err(UrlRewriteError::CheckpointMetadataParseFailed)?;
            let Some(checkpoint) = self.find_checkpoint_by_etag(&checkpoint_metadata, original_etag) else {
                *components_or_url = Either::Right(rule.url_prefix_from.format_with_suffix(
                    path_suffix,
                    Some(original_query),
                    components.fragment(),
                ));
                continue;
            };

            *components_or_url = Either::Right(
                rule.url_prefix_from
                    .format_with_checkpoint_suffix(path_suffix, checkpoint),
            );
            continue;
        }

        Ok(components_or_urls
            .into_iter()
            .map(|components_or_url| match components_or_url {
                Either::Left((components, _)) => components.into_inner(),
                Either::Right(url) => url,
            })
            .collect())
    }

    /// Resolve a resource identity by the checkpoint etag.
    async fn resolve_resorce_identity_to_checkpoint_id(
        &self,
        address: &str,
        etag: &str,
        mut cache: Option<&mut CheckpointMetadataCache>,
    ) -> Result<Option<u64>, UrlRewriteError> {
        if let Some(cache) = cache.as_mut() {
            if cache.last_resource_address.as_deref() == Some(address) {
                return Ok(self.find_checkpoint_by_etag(&cache.last_metadata_entries, etag));
            }
        }

        let metadata_entries = self.fetch_checkpoint_metadata(address).await?;
        let checkpoint_id = self.find_checkpoint_by_etag(&metadata_entries, etag);

        if let Some(cache) = cache.as_mut() {
            cache.last_resource_address = Some(address.into());
            cache.last_metadata_entries = metadata_entries;
        }

        Ok(checkpoint_id)
    }

    /// Find checkpoint id by its etag.
    fn find_checkpoint_by_etag(&self, metadata: &[CheckpointMetadata], etag: &str) -> Option<u64> {
        metadata
            .iter()
            .filter_map(|CheckpointMetadata { id, resource_identity }| {
                if let Some(checkpoint_etag) = split_query_from_url(resource_identity)
                    .1
                    .map(get_etag_query_parameter)
                    .unwrap_or_default()
                {
                    if etag == checkpoint_etag {
                        Some(*id)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .next()
    }

    /// Fetch checkpoint metadata for the specified URI.
    async fn fetch_checkpoint_metadata(
        &self,
        resource_address: impl Into<String>,
    ) -> Result<Vec<CheckpointMetadata>, UrlRewriteError> {
        use crate::backend::metadata::GetMetadataBatchRequest;

        let Some(metadata) = self.metadata.as_ref() else {
            return Ok(Default::default());
        };

        let result = metadata
            .get_metadata_batch(Request::new(GetMetadataBatchRequest {
                requests: vec![self.create_batch_get_request_entry(resource_address)],
            }))
            .await
            .map_err(UrlRewriteError::fetch_failed)?;

        debug_assert_eq!(result.results.len(), 1);

        let entry = result
            .results
            .into_iter()
            .next()
            .ok_or_else(|| UrlRewriteError::BatchResponseEntryMissing)?;
        let Some(value) = get_user_metadata_value(entry)? else {
            return Ok(Default::default());
        };

        CheckpointMetadata::parse_all(value).map_err(UrlRewriteError::CheckpointMetadataParseFailed)
    }

    /// Create a request entry for a GetMetadataBatch request.
    fn create_batch_get_request_entry(&self, resource_address: impl Into<String>) -> GetMetadataBatchRequestEntry {
        use crate::backend::metadata::{resource_location::Resource, GetMetadataBatchRequestEntry, ResourceLocation};

        GetMetadataBatchRequestEntry {
            resource_location: Some(ResourceLocation {
                resource: Some(Resource::ResourceAddress(resource_address.into())),
            }),
            user_metadata_key: self.checkpoint_metadata_key.clone(),
            namespace: None,
        }
    }

    fn synthetic_children_of(&self, parent_url: &str) -> Vec<String> {
        self.synthetic_index.synthetic_children_of(parent_url)
    }

    fn is_mount_point(&self, url: &str) -> bool {
        let Ok(components) = UrlComponents::parse(url) else {
            return false;
        };
        for rule in self.rewrite_rules.iter() {
            let stripped = rule.url_prefix_from.strip_from(
                components.scheme(),
                components.authority(),
                components.path(),
                RewriteMode::Fuzzy,
            );
            if matches!(stripped, Some("")) {
                return true;
            }
        }
        false
    }
}

impl<T: Debug> Debug for UrlRemapperInner<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UrlMapperInner")
            .field("metadata", &self.metadata)
            .field("rewrite_rules", &self.rewrite_rules)
            .field("checkpoint_metadata_key", &self.checkpoint_metadata_key)
            .finish()
    }
}

/// URL components of a URL prefix.
#[derive(Debug, Default, Clone)]
struct UrlPrefixComponents {
    scheme: String,
    authority: String,
    encoded_path: String,
    decoded_path: Vec<u8>,
}

impl UrlPrefixComponents {
    /// Parse components from a URL string.
    fn parse(url: &str) -> Result<Self, UrlParseError> {
        let url_components = UrlComponents::parse(url).map_err(|(_, error)| error)?;
        if url_components.query().is_some() {
            return Err(UrlParseError::UnexpectedQuery);
        }

        if url_components.fragment().is_some() {
            return Err(UrlParseError::UnexpectedFragment);
        }

        let path = url_components.path();
        if !path.ends_with("/") {
            return Err(UrlParseError::TrailingSlashMissing);
        }

        let decoded_path: Cow<[u8]> = percent_decode(path.as_bytes()).into();

        Ok(Self {
            scheme: url_components.scheme().into(),
            authority: url_components.authority().into(),
            decoded_path: decoded_path.into_owned(),
            encoded_path: path.into(),
        })
    }

    /// Strip this prefix from a url with the specified scheme, authority, and path. Return the remaining path.
    fn strip_from<'a>(&self, scheme: &str, authority: &str, path: &'a str, mode: RewriteMode) -> Option<&'a str> {
        debug_assert!(self.encoded_path.ends_with("/"));
        debug_assert!(self.encoded_path.starts_with("/"));
        debug_assert!(self.decoded_path.ends_with(b"/"));
        debug_assert!(self.decoded_path.starts_with(b"/"));

        if scheme == self.scheme && authority == self.authority {
            percent_decode_and_strip_prefix(path, &self.decoded_path, mode)
        } else {
            None
        }
    }

    /// Rewrite the prefix for a matching suffix.
    fn format_with_suffix(&self, path_suffix: &str, query: Option<&str>, fragment: Option<&str>) -> String {
        debug_assert!(self.encoded_path.ends_with("/"));
        debug_assert!(self.encoded_path.starts_with("/"));
        debug_assert!(self.decoded_path.ends_with(b"/"));
        debug_assert!(self.decoded_path.starts_with(b"/"));

        format!(
            "{}://{}{}{}{}{}",
            self.scheme,
            self.authority,
            self.encoded_path,
            path_suffix,
            MaybeQuery(query),
            MaybeFragment(fragment)
        )
    }

    /// Rewrite the prefix for a matching checkpointed suffix.
    fn format_with_checkpoint_suffix(&self, path_suffix: &str, checkpoint: u64) -> String {
        debug_assert!(self.encoded_path.ends_with("/"));
        debug_assert!(self.encoded_path.starts_with("/"));
        debug_assert!(self.decoded_path.ends_with(b"/"));
        debug_assert!(self.decoded_path.starts_with(b"/"));

        format!(
            "{}://{}{}{}?&{}",
            self.scheme, self.authority, self.encoded_path, path_suffix, checkpoint
        )
    }
}

/// Reject configurations where a rule source is more than one path segment
/// deeper than the deepest matching ancestor rule source. Top-level mounts
/// (path == `/`) carry no parent requirement; every other mount must have
/// an explicit rewrite rule at the path one segment shallower, unless it is
/// hosted directly by a configured backend endpoint (a strict descendant of
/// an endpoint root with no intervening alias rule).
///
/// Catches misconfigurations like `omniverse://main/public-assets/1/` when
/// `omniverse://main/` exists but `omniverse://main/public-assets/` does
/// not -- the latter would otherwise surface as a synthetic folder users
/// could click into and find empty / errored, because no rule actually
/// resolves it to a real backend.
fn validate_rule_hierarchy(rules: &[RewriteRule], endpoint_roots: &EndpointRoots) -> Result<(), ConfigError> {
    let sources: std::collections::HashSet<(&str, &str, &str)> = rules
        .iter()
        .map(|r| {
            (
                r.url_prefix_from.scheme.as_str(),
                r.url_prefix_from.authority.as_str(),
                r.url_prefix_from.encoded_path.as_str(),
            )
        })
        .collect();
    for rule in rules {
        let from = &rule.url_prefix_from;
        let Some(parent_path) = parent_encoded_path(&from.encoded_path) else {
            continue;
        };
        let parent_is_rule = sources.contains(&(from.scheme.as_str(), from.authority.as_str(), parent_path.as_str()));
        // The endpoint exemption applies only to a mount hosted *directly* by an endpoint: an
        // ancestor alias shadows the path, so the endpoint no longer supplies the intermediate
        // folders and a skipped level under it would be an orphaned synthetic mount.
        let hosted_directly_by_endpoint = endpoint_roots.hosts(rule) && !has_ancestor_rule(rule, rules);
        if !parent_is_rule && !hosted_directly_by_endpoint {
            return Err(ConfigError::RuleParentNotMounted {
                rule_source: format!("{}://{}{}", from.scheme, from.authority, from.encoded_path),
                missing_parent: format!("{}://{}{}", from.scheme, from.authority, parent_path),
            });
        }
    }
    Ok(())
}

/// Whether a backend endpoint's own root URL is itself returned by `ListTopLevelAddresses` as a
/// browsable top-level address. This decides whether a mount hosted directly by the endpoint is
/// suppressed from the top-level listing: a mount under a [`SelfListed`](Self::SelfListed) root is
/// already reachable by browsing into that root, so it is dropped as a standalone entry; a mount
/// under a [`HostingOnly`](Self::HostingOnly) root -- a hidden endpoint, or an Azure account root
/// the adapter expands into containers rather than listing directly -- is not browsable, so it
/// keeps its own entry or would be undiscoverable. Either kind hosts mounts for hierarchy
/// validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootListing {
    SelfListed,
    HostingOnly,
}

/// The physical backend endpoint roots injected into the remapper -- the single source of truth for
/// what each endpoint hosts. Two behaviors flow from it: hierarchy validation
/// ([`validate_rule_hierarchy`]) exempts a directly-hosted mount from the parent-alias requirement
/// via [`hosts`](Self::hosts), using *every* root because even a hidden endpoint structurally
/// supplies the intermediate folders; and top-level listing ([`UrlRemapper::top_level_rewrite_rules`])
/// suppresses a mount that a self-listed root already exposes as a browsable folder via
/// [`self_listed_hosts`](Self::self_listed_hosts).
#[derive(Debug, Default, Clone)]
struct EndpointRoots {
    roots: Vec<EndpointRoot>,
}

/// A single backend endpoint root and whether its own URL is a browsable top-level address.
#[derive(Debug, Clone)]
struct EndpointRoot {
    prefix: UrlPrefixComponents,
    listing: RootListing,
}

impl EndpointRoots {
    /// Parse endpoint roots from configuration. Roots come from already-validated endpoint
    /// configuration; a root that does not parse simply cannot host a mount, so it is skipped
    /// rather than failing startup.
    fn from_config<S: AsRef<str>>(roots: impl IntoIterator<Item = (S, RootListing)>) -> Self {
        let roots = roots
            .into_iter()
            .filter_map(|(root, listing)| {
                Some(EndpointRoot {
                    prefix: UrlPrefixComponents::parse(root.as_ref()).ok()?,
                    listing,
                })
            })
            .collect();
        Self { roots }
    }

    /// Whether `rule`'s source is a mount hosted directly by *any* configured endpoint root. Even a
    /// hidden endpoint structurally hosts a mount nested under it, so hierarchy validation uses this.
    fn hosts(&self, rule: &RewriteRule) -> bool {
        self.roots.iter().any(|root| Self::root_hosts(root, rule))
    }

    /// Whether `rule`'s source is a mount hosted directly by a *self-listed* endpoint root -- one
    /// whose own root URL is a browsable top-level address, so the mount is already reachable by
    /// browsing into it. Top-level listing uses this to suppress such a mount as a standalone entry.
    fn self_listed_hosts(&self, rule: &RewriteRule) -> bool {
        self.roots
            .iter()
            .any(|root| root.listing == RootListing::SelfListed && Self::root_hosts(root, rule))
    }

    /// Whether `rule`'s source is a strict descendant of `root`, i.e. the mount is hosted directly
    /// by this real endpoint rather than by a parent alias.
    fn root_hosts(root: &EndpointRoot, rule: &RewriteRule) -> bool {
        let from = &rule.url_prefix_from;
        // Case-sensitive, matching the rest of the module (`strip_from`, `has_ancestor_rule`, and
        // the synthetic-children index all compare scheme/authority verbatim). Making only this
        // check case-insensitive would accept a mixed-case mount that the index and rewrite path
        // then fail to surface, losing every advertised access path; a mismatch is instead rejected
        // loudly at startup as `RuleParentNotMounted`.
        root.prefix.scheme == from.scheme
            && root.prefix.authority == from.authority
            && from.encoded_path.len() > root.prefix.encoded_path.len()
            && from.encoded_path.starts_with(&root.prefix.encoded_path)
    }
}

/// Strip the last path segment of `/`-bracketed `encoded_path`. Returns
/// `None` when `encoded_path` is the authority-root path `/` (top-level
/// mount, no parent to require). The input is expected to start and end
/// with `/` -- `UrlPrefixComponents::parse` guarantees that shape.
///
/// Examples: `/` -> None; `/a/` -> `/`; `/a/b/c/` -> `/a/b/`.
fn parent_encoded_path(encoded_path: &str) -> Option<String> {
    let trimmed = encoded_path.strip_suffix('/').unwrap_or(encoded_path);
    let last_slash = trimmed.rfind('/')?;
    Some(trimmed[..=last_slash].to_owned())
}

// Rule source paths always start and end with `/`, so a strictly shorter prefix path is an ancestor.
fn has_ancestor_rule(rule: &RewriteRule, rules: &[RewriteRule]) -> bool {
    let from = &rule.url_prefix_from;
    rules.iter().any(|other| {
        let ancestor = &other.url_prefix_from;
        ancestor.scheme == from.scheme
            && ancestor.authority == from.authority
            && ancestor.encoded_path.len() < from.encoded_path.len()
            && from.encoded_path.starts_with(&ancestor.encoded_path)
    })
}

/// URL components.
#[derive(Debug)]
struct UrlComponents<T> {
    inner: T,
    offsets: UrlComponentsOffsets,
}

impl<T: AsRef<str>> UrlComponents<T> {
    fn into_inner(self) -> T {
        self.inner
    }

    /// Return the scheme component.
    fn scheme(&self) -> &str {
        self.substr(..self.offsets.scheme_end)
    }

    /// Return the authority component.
    fn authority(&self) -> &str {
        self.substr(self.offsets.scheme_end + 3..self.offsets.authority_end)
    }

    /// Return the path component.
    fn path(&self) -> &str {
        if let Some(start) = self.offsets.path_start {
            if let Some(end) = self.offsets.query_start {
                self.substr(start..end - 1)
            } else if let Some(end) = self.offsets.fragment_start {
                self.substr(start..end - 1)
            } else {
                self.substr(start..)
            }
        } else {
            "/"
        }
    }

    /// Return the resource url, without the query and fragment components.
    fn resource_url(&self) -> &str {
        if let Some(pos) = self.offsets.query_start {
            self.substr(..pos)
        } else if let Some(pos) = self.offsets.fragment_start {
            self.substr(..pos)
        } else {
            self.inner.as_ref()
        }
    }

    /// Return the query component.
    fn query(&self) -> Option<&str> {
        if let Some(start) = self.offsets.query_start {
            if let Some(end) = self.offsets.fragment_start {
                Some(self.substr(start..end - 1))
            } else {
                Some(self.substr(start..))
            }
        } else {
            None
        }
    }

    /// Return the fragment component.
    fn fragment(&self) -> Option<&str> {
        if let Some(start) = self.offsets.fragment_start {
            Some(self.substr(start..))
        } else {
            None
        }
    }

    /// Split a URL string into components, without validation. Just enough for the remapper purposes.
    /// <scheme>://<authority>(/<path>?)?(?<query>)?(#<fragment>)?
    fn parse(url: T) -> Result<Self, (T, UrlParseError)> {
        match UrlComponentsOffsets::from_iter(url.as_ref().bytes()) {
            Ok(offsets) => Ok(Self { offsets, inner: url }),
            Err(error) => Err((url, error)),
        }
    }

    fn substr(&self, index: impl SliceIndex<str, Output = str>) -> &str {
        self.inner.as_ref().index(index)
    }
}

#[derive(Default, Debug)]
struct UrlComponentsOffsets {
    scheme_end: usize,
    authority_end: usize,
    path_start: Option<usize>,
    query_start: Option<usize>,
    fragment_start: Option<usize>,
}

impl UrlComponentsOffsets {
    fn from_iter(iter: impl IntoIterator<Item = u8>) -> Result<Self, UrlParseError> {
        let mut state = Self::default();
        state.continue_with_scheme(iter.into_iter().enumerate())?;

        Ok(state)
    }

    /// Parse the rest of a url starting with scheme.
    fn continue_with_scheme(&mut self, mut iter: impl Iterator<Item = (usize, u8)>) -> Result<(), UrlParseError> {
        while let Some((pos, b)) = iter.next() {
            if b == b':' {
                if pos == 0 {
                    return Err(UrlParseError::EmptyScheme);
                }

                for _ in 0..2 {
                    let maybe_slash = iter.next().ok_or(UrlParseError::MissingSchemeSeparator)?;
                    if maybe_slash.1 != b'/' {
                        return Err(UrlParseError::MissingSchemeSeparator);
                    }
                }

                self.scheme_end = pos;

                return self.continue_with_authority(iter);
            }
        }

        Err(UrlParseError::MissingSchemeSeparator)
    }

    /// Parse the the rest of a url starting with authority.
    fn continue_with_authority(&mut self, mut iter: impl Iterator<Item = (usize, u8)>) -> Result<(), UrlParseError> {
        let mut last_pos = None;
        while let Some((pos, b)) = iter.next() {
            if b == b'/' {
                last_pos.ok_or(UrlParseError::EmptyAuthority)?;

                self.authority_end = pos;
                self.path_start = Some(pos);

                return self.continue_with_path(iter);
            } else if b == b'?' {
                last_pos.ok_or(UrlParseError::EmptyAuthority)?;

                self.authority_end = pos;
                self.query_start = Some(pos + 1);

                return self.continue_with_query(iter);
            } else if b == b'#' {
                last_pos.ok_or(UrlParseError::EmptyAuthority)?;

                self.authority_end = pos;
                self.fragment_start = Some(pos + 1);

                return Ok(());
            }

            last_pos = Some(pos);
        }

        self.authority_end = last_pos.ok_or(UrlParseError::EmptyAuthority)? + 1;

        Ok(())
    }

    /// Parse the rest of a url starting with path.
    fn continue_with_path(&mut self, mut iter: impl Iterator<Item = (usize, u8)>) -> Result<(), UrlParseError> {
        while let Some((pos, b)) = iter.next() {
            if b == b'?' {
                self.query_start = Some(pos + 1);

                return self.continue_with_query(iter);
            } else if b == b'#' {
                self.fragment_start = Some(pos + 1);

                return Ok(());
            }
        }

        Ok(())
    }

    /// Parse the rest of a url starting with query.
    fn continue_with_query(&mut self, iter: impl Iterator<Item = (usize, u8)>) -> Result<(), UrlParseError> {
        for (pos, b) in iter {
            if b == b'#' {
                self.fragment_start = Some(pos + 1);

                return Ok(());
            }
        }

        Ok(())
    }
}

/// Writes a query string with the separator, if any.
struct MaybeQuery<'a>(Option<&'a str>);

impl std::fmt::Display for MaybeQuery<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(query) = self.0 {
            write!(f, "?{}", query)
        } else {
            Ok(())
        }
    }
}

/// Writes a fragment string with the separator, if any.
struct MaybeFragment<'a>(Option<&'a str>);

impl std::fmt::Display for MaybeFragment<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(fragment) = self.0 {
            write!(f, "#{}", fragment)
        } else {
            Ok(())
        }
    }
}

trait ParseWarnExt {
    fn inspect_invalid_request_url(self) -> Self;
    fn inspect_invalid_response_url(self) -> Self;
}

impl<R, U, E> ParseWarnExt for Result<R, (U, E)>
where
    U: Debug,
    E: std::error::Error + 'static,
{
    #[track_caller]
    fn inspect_invalid_request_url(self) -> Self {
        self.inspect_err(
            |(url, error)| warn!(?url, error = %error.pretty(), "request url is invalid and won't be remapped"),
        )
    }

    #[track_caller]
    fn inspect_invalid_response_url(self) -> Self {
        self.inspect_err(
            |(url, error)| warn!(?url, error = %error.pretty(), "response url is invalid and won't be remapped"),
        )
    }
}

/// Percent decode the source string and try stripping the specified prefix.
fn percent_decode_and_strip_prefix<'a>(value: &'a str, prefix: &[u8], mode: RewriteMode) -> Option<&'a str> {
    let mut prefix_iter = prefix.iter().copied();
    let mut value_iter = value.as_bytes().iter().copied();
    let mut value_start = 0usize;
    loop {
        let Some(prefix_byte) = prefix_iter.next() else {
            return Some(&value[value_start..]);
        };

        let Some((value_byte, value_bytes_read)) = percent_decode_next(&mut value_iter) else {
            if matches!(mode, RewriteMode::Fuzzy) && prefix_byte == b'/' && prefix_iter.next().is_none() {
                return Some("");
            }

            return None;
        };

        if prefix_byte != value_byte {
            return None;
        }

        value_start += value_bytes_read;
    }
}

/// Decode the next byte in a sequence and return the number of bytes read.
fn percent_decode_next(iter: &mut (impl Iterator<Item = u8> + Clone)) -> Option<(u8, usize)> {
    let maybe_percent = iter.next()?;
    if maybe_percent == b'%' {
        if let Some(decoded) = percent_decode_next_hex(iter) {
            return Some((decoded, 3));
        }
    }

    Some((maybe_percent, 1))
}

/// Decode a hexadecimal sequence of bytes.
fn percent_decode_next_hex(iter: &mut (impl Iterator<Item = u8> + Clone)) -> Option<u8> {
    let mut cloned = iter.clone();
    let h = char::from(cloned.next()?).to_digit(16)?;
    let l = char::from(cloned.next()?).to_digit(16)?;

    *iter = cloned;

    Some(h as u8 * 0x10 + l as u8)
}

fn get_user_metadata_value(entry: GetMetadataBatchResponseEntry) -> Result<Option<Value>, UrlRewriteError> {
    use crate::backend::metadata::{batch_error_entry::Code, get_metadata_batch_response_entry::Outcome};

    match entry
        .outcome
        .ok_or_else(|| UrlRewriteError::BatchResponseEntryOutcomeMissing)?
    {
        Outcome::Success(value) => Ok(Some(
            value
                .value
                .ok_or_else(|| UrlRewriteError::BatchResponseEntryValueMissing)?,
        )),
        Outcome::Error(error) => {
            if error.code == Code::NotFound as i32 || error.code == Code::InvalidArgument as i32 {
                Ok(None)
            } else {
                Err(UrlRewriteError::BatchRequestFailed(error.code, error.message))
            }
        },
    }
}

#[cfg(test)]
pub(crate) mod test_helpers {
    use std::collections::HashMap;

    use tonic::async_trait;

    use crate::{
        backend::metadata::Metadata,
        common::url_remapper::{RewriteRuleConfig, RootListing, UrlRemapper, UrlRemapperConfig},
    };

    #[derive(Debug, Clone)]
    pub struct UnreachableMetadata;

    #[async_trait]
    impl Metadata for UnreachableMetadata {
        fn inner(&self) -> Option<&dyn Metadata> {
            None
        }
    }

    pub fn make_remapper(rules: &[(&str, &str)], enabled: bool) -> UrlRemapper<UnreachableMetadata> {
        make_remapper_full(rules, &[], enabled)
    }

    pub fn make_remapper_with_endpoints(
        rules: &[(&str, &str)],
        enabled: bool,
        endpoint_roots: &[&str],
    ) -> UrlRemapper<UnreachableMetadata> {
        make_remapper_full(rules, endpoint_roots, enabled)
    }

    fn make_remapper_full(
        rules: &[(&str, &str)],
        endpoint_roots: &[&str],
        enabled: bool,
    ) -> UrlRemapper<UnreachableMetadata> {
        let rewrite_rules: HashMap<String, RewriteRuleConfig> = rules
            .iter()
            .map(|(k, v)| {
                (
                    String::from(*k),
                    RewriteRuleConfig {
                        target: String::from(*v),
                        list_in_top_level_addresses: true,
                    },
                )
            })
            .collect();
        let config = UrlRemapperConfig {
            enabled: true,
            rewrite_rules,
            checkpoint_metadata_key: "checkpoints".into(),
            expose_child_rules_as_folders: enabled,
        };
        // The test helper treats every endpoint as advertised; hidden-endpoint behavior is covered
        // by dedicated tests that call `new` directly.
        UrlRemapper::<UnreachableMetadata>::new(
            None,
            config,
            endpoint_roots.iter().map(|&root| (root, RootListing::SelfListed)),
        )
        .expect("config parses")
        .expect("remapper enabled")
    }
}

#[cfg(test)]
mod tests {
    use crate::common::url_remapper::{
        error::{ConfigError, UrlParseError},
        percent_decode_and_strip_prefix, RewriteMode, RewriteRuleConfig, RootListing, UrlComponents,
        UrlPrefixComponents, UrlRemapper, UrlRemapperConfig,
    };
    use std::collections::HashMap;

    use crate::common::url_remapper::test_helpers::{make_remapper, make_remapper_with_endpoints, UnreachableMetadata};

    use rstest::rstest;

    fn top_level_addresses(mapper: &UrlRemapper<UnreachableMetadata>) -> Vec<String> {
        let mut addresses: Vec<String> = mapper
            .top_level_rewrite_rules()
            .map(|rule| rule.get_top_level_address_entry_v1alpha().top_level_address)
            .collect();
        addresses.sort();
        addresses
    }

    #[rstest]
    #[case("https://myhost.mydomain", "https", "myhost.mydomain", "/", None, None)]
    #[case("https://myhost.mydomain/", "https", "myhost.mydomain", "/", None, None)]
    #[case("https://myhost.mydomain//", "https", "myhost.mydomain", "//", None, None)]
    #[case("https://myhost.mydomain?q", "https", "myhost.mydomain", "/", Some("q"), None)]
    #[case("https://myhost.mydomain?", "https", "myhost.mydomain", "/", Some(""), None)]
    #[case("https://myhost.mydomain/?q", "https", "myhost.mydomain", "/", Some("q"), None)]
    #[case("https://myhost.mydomain#f", "https", "myhost.mydomain", "/", None, Some("f"))]
    #[case("https://myhost.mydomain#", "https", "myhost.mydomain", "/", None, Some(""))]
    #[case("https://myhost.mydomain/#f", "https", "myhost.mydomain", "/", None, Some("f"))]
    #[case("https://myhost.mydomain/#", "https", "myhost.mydomain", "/", None, Some(""))]
    #[case("https://myhost.mydomain/#f?q", "https", "myhost.mydomain", "/", None, Some("f?q"))]
    #[case(
        "https://myhost.mydomain//#/f?q",
        "https",
        "myhost.mydomain",
        "//",
        None,
        Some("/f?q")
    )]
    #[case(
        "https://myhost.mydomain/?q/#/f?q",
        "https",
        "myhost.mydomain",
        "/",
        Some("q/"),
        Some("/f?q")
    )]
    fn test_url_components_ref_parse(
        #[case] source: &str,
        #[case] expected_scheme: &str,
        #[case] expected_authority: &str,
        #[case] expected_path: &str,
        #[case] expected_query: Option<&str>,
        #[case] expected_fragment: Option<&str>,
    ) {
        let components = UrlComponents::parse(source).expect("failed to parse a url");

        assert_eq!(components.scheme(), expected_scheme);
        assert_eq!(components.authority(), expected_authority);
        assert_eq!(components.path(), expected_path);
        assert_eq!(components.query(), expected_query);
        assert_eq!(components.fragment(), expected_fragment);
    }

    #[rstest]
    #[case("", UrlParseError::MissingSchemeSeparator)]
    #[case("scheme", UrlParseError::MissingSchemeSeparator)]
    #[case("scheme:", UrlParseError::MissingSchemeSeparator)]
    #[case("scheme:/", UrlParseError::MissingSchemeSeparator)]
    #[case("://", UrlParseError::EmptyScheme)]
    #[case("scheme://", UrlParseError::EmptyAuthority)]
    #[case("scheme:///", UrlParseError::EmptyAuthority)]
    #[case("scheme://?", UrlParseError::EmptyAuthority)]
    #[case("scheme://#", UrlParseError::EmptyAuthority)]
    fn test_url_components_ref_parse_error(#[case] source: &str, #[case] expected_error: UrlParseError) {
        let error = UrlComponents::parse(source).expect_err("url components parse is expected to fail");

        assert_eq!(error.1, expected_error);
    }

    #[rstest]
    #[case("stuff", None, None, "https://hostname/prefix/stuff")]
    #[case("stuff", Some("query"), None, "https://hostname/prefix/stuff?query")]
    #[case("stuff", None, Some("fragment"), "https://hostname/prefix/stuff#fragment")]
    #[case(
        "stuff",
        Some("query"),
        Some("fragment"),
        "https://hostname/prefix/stuff?query#fragment"
    )]
    fn test_url_prefix_components_format_with_suffix(
        #[case] path_suffix: &str,
        #[case] query: Option<&str>,
        #[case] fragment: Option<&str>,
        #[case] expected_url: &str,
    ) {
        let components =
            UrlPrefixComponents::parse("https://hostname/prefix/").expect("failed to parse prefix components");

        assert_eq!(
            components.format_with_suffix(path_suffix, query, fragment),
            expected_url
        );
    }

    #[rstest]
    #[case("https", "host", "/path/", RewriteMode::Precise, None)]
    #[case("scheme", "myhost", "/path/", RewriteMode::Precise, None)]
    #[case("scheme", "host", "/path", RewriteMode::Precise, None)]
    #[case("scheme", "host", "/path/", RewriteMode::Precise, Some(""))]
    #[case("scheme", "host", "/path/to/object", RewriteMode::Precise, Some("to/object"))]
    #[case("scheme", "host", "/path/to/folder/", RewriteMode::Fuzzy, Some("to/folder/"))]
    #[case("scheme", "host", "/path", RewriteMode::Fuzzy, Some(""))]
    #[case("scheme", "host", "/path/to/object", RewriteMode::Fuzzy, Some("to/object"))]
    #[case("scheme", "host", "/path/to/folder/", RewriteMode::Fuzzy, Some("to/folder/"))]
    fn test_url_prefix_components_strip_prefix(
        #[case] scheme: &str,
        #[case] authority: &str,
        #[case] path: &str,
        #[case] mode: RewriteMode,
        #[case] expected: Option<&str>,
    ) {
        let components = UrlPrefixComponents::parse("scheme://host/path/").expect("failed to parse prefix components");

        assert_eq!(components.strip_from(scheme, authority, path, mode), expected);
    }

    #[rstest]
    #[case("%2fa%2fb", b"/a/b/", RewriteMode::Precise, None)]
    #[case("%2fa%2fb", b"/a/b/", RewriteMode::Fuzzy, Some(""))]
    #[case("%2fa%2fb/c", b"/a/b/", RewriteMode::Precise, Some("c"))]
    #[case("%2fa%2fb%2Fc", b"/a/b/", RewriteMode::Fuzzy, Some("c"))]
    #[case("%2fa%2fb%2Fc%20", b"/a/b/", RewriteMode::Precise, Some("c%20"))]
    #[case("%2fa%2fb%2F", b"/a/b/", RewriteMode::Precise, Some(""))]
    #[case("/a/b/", b"a/b/", RewriteMode::Fuzzy, None)]
    #[case("/a/b", b"/a/b/c", RewriteMode::Fuzzy, None)]
    #[case("/a/b", b"/a/bc", RewriteMode::Fuzzy, None)]
    #[case("/a/%2q!!", b"/a/%2q", RewriteMode::Precise, Some("!!"))]
    #[case("/a/%2q!!", b"/a/%2q", RewriteMode::Fuzzy, Some("!!"))]
    #[case("/a/%2b!!", b"/a/%2b", RewriteMode::Precise, None)]
    #[case("/a/%2b!!", b"/a/%2b", RewriteMode::Fuzzy, None)]
    #[case("", b"/", RewriteMode::Fuzzy, Some(""))]
    fn test_percent_decode_and_strip_prefix(
        #[case] value: &str,
        #[case] prefix: &[u8],
        #[case] mode: RewriteMode,
        #[case] expected: Option<&str>,
    ) {
        let decoded = percent_decode_and_strip_prefix(value, prefix, mode);

        assert_eq!(decoded, expected);
    }

    /// Feature flag off: no synthetic children regardless of configuration.
    #[test]
    fn synthetic_children_off_when_disabled() {
        let mapper = make_remapper(
            &[
                ("omniverse://host/", "https://backend.example/bucket-a/"),
                ("omniverse://host/Folder/", "https://backend.example/bucket-b/"),
            ],
            false,
        );
        let children = mapper.synthetic_children_of("omniverse://host/");
        assert!(children.is_empty());
    }

    /// Basic case: a single child rule under an omniverse:// parent.
    #[test]
    fn synthetic_children_omniverse_parent() {
        let mapper = make_remapper(
            &[
                ("omniverse://host/", "https://backend.example/bucket-a/"),
                ("omniverse://host/Folder/", "https://backend.example/bucket-b/"),
            ],
            true,
        );
        let children = mapper.synthetic_children_of("omniverse://host/");
        assert_eq!(children, vec!["omniverse://host/Folder/".to_string()]);
    }

    /// Native cloud URL parent: a rule sitting under a real backend URL
    /// surfaces as a synthetic folder when that URL is listed. The
    /// parent URL must itself be a remap rule for the config to validate.
    #[test]
    fn synthetic_children_native_cloud_parent() {
        let mapper = make_remapper(
            &[
                (
                    "https://account.blob.core.windows.net/",
                    "https://account.blob.core.windows.net/root-target/",
                ),
                (
                    "https://account.blob.core.windows.net/bucket-a/",
                    "https://account.blob.core.windows.net/bucket-a-target/",
                ),
                (
                    "https://account.blob.core.windows.net/bucket-a/Folder/",
                    "https://account.blob.core.windows.net/bucket-b/",
                ),
            ],
            true,
        );
        let children = mapper.synthetic_children_of("https://account.blob.core.windows.net/bucket-a/");
        assert_eq!(
            children,
            vec!["https://account.blob.core.windows.net/bucket-a/Folder/".to_string()]
        );
    }

    /// Multi-level mount: rules at A/ and A/B/ collapse to the first
    /// direct-child segment when listing the grandparent.
    #[test]
    fn synthetic_children_multi_level_collapses_to_direct_child() {
        let mapper = make_remapper(
            &[
                ("omniverse://host/", "https://backend.example/root/"),
                ("omniverse://host/A/", "https://backend.example/a/"),
                ("omniverse://host/A/B/", "https://backend.example/b/"),
            ],
            true,
        );
        let children_of_root = mapper.synthetic_children_of("omniverse://host/");
        assert_eq!(children_of_root, vec!["omniverse://host/A/".to_string()]);

        let children_of_a = mapper.synthetic_children_of("omniverse://host/A/");
        assert_eq!(children_of_a, vec!["omniverse://host/A/B/".to_string()]);
    }

    /// Cross-scheme isolation: an `omniverse://` parent must not pick up
    /// rules whose source uses `https://` with the same path prefix, and
    /// vice versa. Both top-level addresses are remapped here so the
    /// `https://host/Folder/` child has a valid parent.
    #[test]
    fn synthetic_children_cross_scheme_isolated() {
        let mapper = make_remapper(
            &[
                ("https://host/", "https://backend.example/host-target/"),
                ("https://host/Folder/", "https://backend.example/bucket-b/"),
            ],
            true,
        );
        assert!(mapper.synthetic_children_of("omniverse://host/").is_empty());
    }

    /// Two rules sharing the same direct-child segment collapse to a
    /// single synthetic entry (not duplicated). Each child rule must
    /// itself sit directly below an existing remap, so the test sets up
    /// the parent chain `omniverse://host/` -> `omniverse://host/Folder/`
    /// before the two sibling leaves.
    #[test]
    fn synthetic_children_dedupes_shared_direct_child() {
        let mapper = make_remapper(
            &[
                ("omniverse://host/", "https://backend.example/root/"),
                ("omniverse://host/Folder/", "https://backend.example/folder/"),
                ("omniverse://host/Folder/sub1/", "https://backend.example/s1/"),
                ("omniverse://host/Folder/sub2/", "https://backend.example/s2/"),
            ],
            true,
        );
        let children = mapper.synthetic_children_of("omniverse://host/");
        assert_eq!(children, vec!["omniverse://host/Folder/".to_string()]);
    }

    /// Listing a parent that exactly equals a rule source returns
    /// children of that rule (not the rule itself), preserving the
    /// "strict descendant" boundary.
    #[test]
    fn synthetic_children_exact_match_excludes_self() {
        let mapper = make_remapper(
            &[
                ("omniverse://host/", "https://backend.example/root/"),
                ("omniverse://host/Folder/", "https://backend.example/b/"),
                ("omniverse://host/Folder/Nested/", "https://backend.example/n/"),
            ],
            true,
        );
        let children = mapper.synthetic_children_of("omniverse://host/Folder/");
        assert_eq!(children, vec!["omniverse://host/Folder/Nested/".to_string()]);
    }

    /// Parent without a trailing slash on a non-root path is still
    /// matched -- the lookup is fuzzy on trailing slashes so callers can
    /// pass either `parent/` or `parent`.
    #[test]
    fn synthetic_children_tolerates_missing_trailing_slash() {
        let mapper = make_remapper(
            &[
                ("omniverse://host/", "https://backend.example/root/"),
                ("omniverse://host/parent/", "https://backend.example/parent/"),
                ("omniverse://host/parent/Folder/", "https://backend.example/b/"),
            ],
            true,
        );
        assert_eq!(
            mapper.synthetic_children_of("omniverse://host/parent"),
            vec!["omniverse://host/parent/Folder/".to_string()]
        );
    }

    /// Consecutive slashes inside a path are valid in S3 / Azure Blob
    /// keys and must not be collapsed by the trie. A rule with `a//b/`
    /// only matches a parent whose decoded path also contains the
    /// empty middle segment.
    #[test]
    fn synthetic_children_preserves_empty_path_segments() {
        let mapper = make_remapper(
            &[
                ("https://host/", "https://backend.example/host-target/"),
                ("https://host/bucket/", "https://backend.example/bucket-target/"),
                ("https://host/bucket/a/", "https://backend.example/a-target/"),
                ("https://host/bucket/a//", "https://backend.example/a-empty-target/"),
                ("https://host/bucket/a//b/", "https://backend.example/ab-target/"),
                ("https://host/bucket/a//b/Folder/", "https://backend.example/b/"),
            ],
            true,
        );
        assert_eq!(
            mapper.synthetic_children_of("https://host/bucket/a//b/"),
            vec!["https://host/bucket/a//b/Folder/".to_string()]
        );
        // A single-slash variant must not match the rule with `a//b`.
        assert!(mapper.synthetic_children_of("https://host/bucket/a/b/").is_empty());
    }

    /// Non-aligned prefixes (e.g. `/foo` vs `/foobar/`) must not match.
    /// The path-segment boundary check is what prevents the false-positive.
    #[test]
    fn synthetic_children_rejects_non_segment_aligned_prefix() {
        let mapper = make_remapper(
            &[
                ("omniverse://host/", "https://backend.example/root/"),
                ("omniverse://host/foobar/", "https://backend.example/foobar/"),
                ("omniverse://host/foobar/sub/", "https://backend.example/s/"),
            ],
            true,
        );
        assert!(mapper.synthetic_children_of("omniverse://host/foo/").is_empty());
    }

    /// A parent URL with extra trailing slashes (e.g. `https://host///`)
    /// must not be collapsed to root; the trie walks `["", ""]` as
    /// meaningful empty segments and finds no match against rules that
    /// don't share that exact shape. Regression test for over-eager
    /// stripping that collapsed `///` to `/`.
    #[test]
    fn synthetic_children_extra_trailing_slashes_not_collapsed() {
        let mapper = make_remapper(
            &[
                ("omniverse://host/", "https://backend.example/root/"),
                ("omniverse://host/Folder/", "https://backend.example/b/"),
            ],
            true,
        );
        // Root parent finds the synthetic child.
        assert_eq!(
            mapper.synthetic_children_of("omniverse://host/"),
            vec!["omniverse://host/Folder/".to_string()]
        );
        // `///` decodes to path `///` which strips one bracketing slash
        // on each side -> inner `"/"` -> segments `["", ""]`. There is
        // no `""` child under the root, so we return empty rather than
        // accidentally aliasing to the bare root.
        assert!(mapper.synthetic_children_of("omniverse://host///").is_empty());
    }

    /// `is_mount_point` returns true for exact rule-source matches and
    /// false for descendants or unrelated URLs. Trailing slash is
    /// optional on the input.
    #[test]
    fn is_mount_point_exact_match_only() {
        let mapper = make_remapper(
            &[
                ("omniverse://host/", "https://backend.example/root/"),
                ("omniverse://host/Folder/", "https://backend.example/b/"),
            ],
            true,
        );
        assert!(mapper.is_mount_point("omniverse://host/Folder/"));
        assert!(mapper.is_mount_point("omniverse://host/Folder"));
        assert!(!mapper.is_mount_point("omniverse://host/Folder/child/"));
        // `omniverse://host/` is a top-level mount and therefore IS a
        // mount-point under the relaxed rules introduced with the
        // direct-child-of-existing-remap validation -- adjust this
        // assertion if you make `is_mount_point` strict about depth.
        assert!(mapper.is_mount_point("omniverse://host/"));
        assert!(!mapper.is_mount_point("omniverse://other/Folder/"));
    }

    /// A config where a non-top-level mount has no direct-parent rule
    /// must be rejected at construction time. Mounts can only be direct
    /// children of an existing remapping.
    #[test]
    fn config_rejects_orphan_non_top_level_mount() {
        use crate::common::url_remapper::test_helpers::UnreachableMetadata;

        let config = UrlRemapperConfig {
            enabled: true,
            rewrite_rules: HashMap::from([
                (
                    "omniverse://main/".to_string(),
                    RewriteRuleConfig {
                        target: "https://some.bucket.com/bucket1/".to_string(),
                        list_in_top_level_addresses: true,
                    },
                ),
                (
                    "omniverse://main/public-assets/1/".to_string(),
                    RewriteRuleConfig {
                        target: "https://some.bucket.com/bucket2/".to_string(),
                        list_in_top_level_addresses: true,
                    },
                ),
            ]),
            checkpoint_metadata_key: "checkpoints".into(),
            expose_child_rules_as_folders: true,
        };
        let err = UrlRemapper::<UnreachableMetadata>::new(None, config, std::iter::empty::<(&str, RootListing)>())
            .expect_err("orphan grandchild mount must be rejected");
        assert!(
            matches!(err, ConfigError::RuleParentNotMounted { .. }),
            "unexpected error variant: {err:?}",
        );
    }

    /// A config where every non-top-level mount has its immediate parent
    /// as another rule must be accepted.
    #[test]
    fn config_accepts_direct_child_mounts() {
        use crate::common::url_remapper::test_helpers::UnreachableMetadata;

        let config = UrlRemapperConfig {
            enabled: true,
            rewrite_rules: HashMap::from([
                (
                    "omniverse://main/".to_string(),
                    RewriteRuleConfig {
                        target: "https://some.bucket.com/bucket1/".to_string(),
                        list_in_top_level_addresses: true,
                    },
                ),
                (
                    "omniverse://main/1/".to_string(),
                    RewriteRuleConfig {
                        target: "https://some.bucket.com/bucket2/".to_string(),
                        list_in_top_level_addresses: true,
                    },
                ),
                (
                    "omniverse://main/2/".to_string(),
                    RewriteRuleConfig {
                        target: "https://some.bucket.com/bucket3/".to_string(),
                        list_in_top_level_addresses: true,
                    },
                ),
            ]),
            checkpoint_metadata_key: "checkpoints".into(),
            expose_child_rules_as_folders: true,
        };
        let mapper = UrlRemapper::<UnreachableMetadata>::new(None, config, std::iter::empty::<(&str, RootListing)>())
            .expect("direct-child mounts validate")
            .expect("remapper enabled");
        // Sanity: both children surface under the top-level parent.
        let mut children = mapper.synthetic_children_of("omniverse://main/");
        children.sort();
        assert_eq!(
            children,
            vec!["omniverse://main/1/".to_string(), "omniverse://main/2/".to_string()],
        );
    }

    /// Validation is opt-in via the same flag that drives synthetic
    /// injection: when `expose_child_rules_as_folders` is off, deep-nested
    /// mounts without their intermediate parents are allowed (they
    /// behave as plain URL rewrites with no synthetic surfacing).
    #[test]
    fn config_skips_hierarchy_check_when_feature_disabled() {
        use crate::common::url_remapper::test_helpers::UnreachableMetadata;

        let config = UrlRemapperConfig {
            enabled: true,
            rewrite_rules: HashMap::from([
                (
                    "omniverse://main/".to_string(),
                    RewriteRuleConfig {
                        target: "https://some.bucket.com/bucket1/".to_string(),
                        list_in_top_level_addresses: true,
                    },
                ),
                (
                    "omniverse://main/public-assets/1/".to_string(),
                    RewriteRuleConfig {
                        target: "https://some.bucket.com/bucket2/".to_string(),
                        list_in_top_level_addresses: true,
                    },
                ),
            ]),
            checkpoint_metadata_key: "checkpoints".into(),
            expose_child_rules_as_folders: false,
        };
        UrlRemapper::<UnreachableMetadata>::new(None, config, std::iter::empty::<(&str, RootListing)>())
            .expect("hierarchy check is gated on the synthetic-folder feature flag");
    }

    /// A mount nested directly under a configured backend endpoint is valid without a covering
    /// alias -- the endpoint is its parent.
    #[test]
    fn config_accepts_mount_under_configured_endpoint() {
        use crate::common::url_remapper::test_helpers::UnreachableMetadata;

        let config = UrlRemapperConfig {
            enabled: true,
            rewrite_rules: HashMap::from([(
                "https://account.blob.core.windows.net/MountedBucket/".to_string(),
                RewriteRuleConfig {
                    target: "https://bucket.s3.us-west-2.amazonaws.com/".to_string(),
                    list_in_top_level_addresses: true,
                },
            )]),
            checkpoint_metadata_key: "checkpoints".into(),
            expose_child_rules_as_folders: true,
        };
        let mapper = UrlRemapper::<UnreachableMetadata>::new(
            None,
            config,
            [("https://account.blob.core.windows.net/", RootListing::SelfListed)],
        )
        .expect("mount under a configured endpoint validates")
        .expect("remapper enabled");
        // The mount surfaces as a synthetic folder when the endpoint root is listed.
        assert_eq!(
            mapper.synthetic_children_of("https://account.blob.core.windows.net/"),
            vec!["https://account.blob.core.windows.net/MountedBucket/".to_string()],
        );
    }

    /// A mount several levels below a configured endpoint (under real folders) is also valid; the
    /// endpoint is still its ancestor.
    #[test]
    fn config_accepts_deep_mount_under_configured_endpoint() {
        use crate::common::url_remapper::test_helpers::UnreachableMetadata;

        let config = UrlRemapperConfig {
            enabled: true,
            rewrite_rules: HashMap::from([(
                "https://account.blob.core.windows.net/a/b/MountedBucket/".to_string(),
                RewriteRuleConfig {
                    target: "https://bucket.s3.us-west-2.amazonaws.com/".to_string(),
                    list_in_top_level_addresses: true,
                },
            )]),
            checkpoint_metadata_key: "checkpoints".into(),
            expose_child_rules_as_folders: true,
        };
        UrlRemapper::<UnreachableMetadata>::new(
            None,
            config,
            [("https://account.blob.core.windows.net/", RootListing::SelfListed)],
        )
        .expect("deep mount under a configured endpoint validates")
        .expect("remapper enabled");
    }

    /// Without a configured endpoint (or a parent alias) the same mount is an orphan and rejected.
    #[test]
    fn config_rejects_mount_under_unconfigured_host() {
        use crate::common::url_remapper::test_helpers::UnreachableMetadata;

        let config = UrlRemapperConfig {
            enabled: true,
            rewrite_rules: HashMap::from([(
                "https://account.blob.core.windows.net/MountedBucket/".to_string(),
                RewriteRuleConfig {
                    target: "https://bucket.s3.us-west-2.amazonaws.com/".to_string(),
                    list_in_top_level_addresses: true,
                },
            )]),
            checkpoint_metadata_key: "checkpoints".into(),
            expose_child_rules_as_folders: true,
        };
        let err = UrlRemapper::<UnreachableMetadata>::new(None, config, std::iter::empty::<(&str, RootListing)>())
            .expect_err("a mount under an unconfigured host must be rejected");
        assert!(
            matches!(err, ConfigError::RuleParentNotMounted { .. }),
            "unexpected error variant: {err:?}",
        );
    }

    /// The endpoint exemption must not apply once an alias shadows the path: a mount that skips a
    /// level under the alias is orphaned, since the alias reroutes that subtree away from the endpoint.
    #[test]
    fn config_rejects_mount_skipping_a_level_under_an_alias() {
        use crate::common::url_remapper::test_helpers::UnreachableMetadata;

        let config = UrlRemapperConfig {
            enabled: true,
            rewrite_rules: HashMap::from([
                (
                    "https://account.blob.core.windows.net/alias/".to_string(),
                    RewriteRuleConfig {
                        target: "https://b.s3.us-west-2.amazonaws.com/".to_string(),
                        list_in_top_level_addresses: true,
                    },
                ),
                (
                    "https://account.blob.core.windows.net/alias/missing/child/".to_string(),
                    RewriteRuleConfig {
                        target: "https://d.s3.us-west-2.amazonaws.com/".to_string(),
                        list_in_top_level_addresses: true,
                    },
                ),
            ]),
            checkpoint_metadata_key: "checkpoints".into(),
            expose_child_rules_as_folders: true,
        };
        let err = UrlRemapper::<UnreachableMetadata>::new(
            None,
            config,
            [("https://account.blob.core.windows.net/", RootListing::SelfListed)],
        )
        .expect_err("a mount skipping a level under an alias must be rejected despite the endpoint");
        assert!(
            matches!(err, ConfigError::RuleParentNotMounted { .. }),
            "unexpected error variant: {err:?}",
        );
    }

    /// Endpoint/rule ancestry is case-sensitive, consistent with the rest of the module. A mount
    /// whose host casing differs from the configured endpoint is not recognized as hosted by it and
    /// is rejected at startup -- a loud, fixable error rather than a silently unreachable mount.
    #[test]
    fn config_rejects_mount_under_endpoint_with_mismatched_host_case() {
        use crate::common::url_remapper::test_helpers::UnreachableMetadata;

        let config = UrlRemapperConfig {
            enabled: true,
            rewrite_rules: HashMap::from([(
                "https://account.blob.core.windows.net/MountedBucket/".to_string(),
                RewriteRuleConfig {
                    target: "https://bucket.s3.us-west-2.amazonaws.com/".to_string(),
                    list_in_top_level_addresses: true,
                },
            )]),
            checkpoint_metadata_key: "checkpoints".into(),
            expose_child_rules_as_folders: true,
        };
        let err = UrlRemapper::<UnreachableMetadata>::new(
            None,
            config,
            [("https://ACCOUNT.blob.core.windows.net/", RootListing::SelfListed)],
        )
        .expect_err("a mismatched-case endpoint host must not be treated as hosting the mount");
        assert!(
            matches!(err, ConfigError::RuleParentNotMounted { .. }),
            "unexpected error variant: {err:?}",
        );
    }

    /// A mount hosted by an endpoint whose own root is a browsable top-level address (an S3 bucket
    /// endpoint, listed verbatim) is reachable by browsing into that root, so it must not also
    /// appear as a standalone top-level address.
    #[test]
    fn top_level_addresses_hide_endpoint_hosted_mount() {
        let mapper = make_remapper_with_endpoints(
            &[(
                "https://bucket.s3.us-west-2.amazonaws.com/MountedFolder/",
                "https://target.s3.us-west-2.amazonaws.com/",
            )],
            true,
            &["https://bucket.s3.us-west-2.amazonaws.com/"],
        );
        assert!(top_level_addresses(&mapper).is_empty());
    }

    /// A mount hosted directly by an Azure storage-account root stays discoverable via its own
    /// top-level entry. The account root is passed as *not* self-listed because the Azure adapter
    /// lists an account's containers rather than the account root itself, so the root is never a
    /// browsable top-level address and cannot surface the mount by browsing into it.
    #[test]
    fn top_level_addresses_show_mount_under_account_root() {
        use crate::common::url_remapper::test_helpers::UnreachableMetadata;

        let mount = "https://account.blob.core.windows.net/MountedBucket/";
        let config = UrlRemapperConfig {
            enabled: true,
            rewrite_rules: HashMap::from([(
                mount.to_string(),
                RewriteRuleConfig {
                    target: "https://bucket.s3.us-west-2.amazonaws.com/".to_string(),
                    list_in_top_level_addresses: true,
                },
            )]),
            checkpoint_metadata_key: "checkpoints".into(),
            expose_child_rules_as_folders: true,
        };
        let mapper = UrlRemapper::<UnreachableMetadata>::new(
            None,
            config,
            [("https://account.blob.core.windows.net/", RootListing::HostingOnly)],
        )
        .expect("mount under a configured account root validates")
        .expect("remapper enabled");

        assert_eq!(top_level_addresses(&mapper), vec![mount.to_string()]);
    }

    /// A mount under an endpoint that is *hidden* from `ListTopLevelAddresses` is not reachable by
    /// browsing into the endpoint, so its explicit `list_in_top_level_addresses: true` must still
    /// surface it -- otherwise the "hide the account, expose the curated mount" config lists neither.
    #[test]
    fn top_level_addresses_show_mount_under_hidden_endpoint() {
        use crate::common::url_remapper::test_helpers::UnreachableMetadata;

        let mount = "https://bucket.s3.us-west-2.amazonaws.com/Public/";
        let config = UrlRemapperConfig {
            enabled: true,
            rewrite_rules: HashMap::from([(
                mount.to_string(),
                RewriteRuleConfig {
                    target: "https://curated.s3.us-west-2.amazonaws.com/".to_string(),
                    list_in_top_level_addresses: true,
                },
            )]),
            checkpoint_metadata_key: "checkpoints".into(),
            expose_child_rules_as_folders: true,
        };
        // The endpoint is configured but hidden (advertised = false), as when the operator sets
        // listInTopLevelAddresses: false on the raw account.
        let mapper = UrlRemapper::<UnreachableMetadata>::new(
            None,
            config,
            [("https://bucket.s3.us-west-2.amazonaws.com/", RootListing::HostingOnly)],
        )
        .expect("mount under a configured endpoint validates")
        .expect("remapper enabled");

        assert_eq!(top_level_addresses(&mapper), vec![mount.to_string()]);
    }

    #[test]
    fn top_level_addresses_hide_nested_mount() {
        let mapper = make_remapper(
            &[
                ("omniverse://content.example.com/", "https://backend.example/bucket-1/"),
                (
                    "omniverse://content.example.com/NVIDIA/",
                    "https://backend.example/bucket-2/",
                ),
            ],
            true,
        );
        assert_eq!(
            top_level_addresses(&mapper),
            vec!["omniverse://content.example.com/".to_string()]
        );
    }

    #[test]
    fn top_level_addresses_keep_only_root_of_mount_tree() {
        let mapper = make_remapper(
            &[
                ("omniverse://content.example.com/", "https://backend.example/bucket-1/"),
                (
                    "omniverse://content.example.com/NVIDIA/",
                    "https://backend.example/bucket-2/",
                ),
                (
                    "omniverse://content.example.com/NVIDIA/Assets/",
                    "https://backend.example/bucket-3/",
                ),
            ],
            true,
        );
        assert_eq!(
            top_level_addresses(&mapper),
            vec!["omniverse://content.example.com/".to_string()]
        );
    }

    #[test]
    fn top_level_addresses_keep_all_when_feature_disabled() {
        let mapper = make_remapper(
            &[
                ("omniverse://content.example.com/", "https://backend.example/bucket-1/"),
                (
                    "omniverse://content.example.com/NVIDIA/",
                    "https://backend.example/bucket-2/",
                ),
            ],
            false,
        );
        assert_eq!(
            top_level_addresses(&mapper),
            vec![
                "omniverse://content.example.com/".to_string(),
                "omniverse://content.example.com/NVIDIA/".to_string(),
            ]
        );
    }

    #[test]
    fn top_level_addresses_keep_independent_roots() {
        let mapper = make_remapper(
            &[
                ("omniverse://host-a/", "https://backend.example/a/"),
                ("omniverse://host-a/sub/", "https://backend.example/a-sub/"),
                ("omniverse://host-b/", "https://backend.example/b/"),
            ],
            true,
        );
        assert_eq!(
            top_level_addresses(&mapper),
            vec!["omniverse://host-a/".to_string(), "omniverse://host-b/".to_string()]
        );
    }
}
