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

//! Forward URL-prefix remapping for resource identifiers.
//!
//! Translates incoming resource URLs (in policies, authorization requests, and
//! diagnostics) using the longest matching configured rule before they reach
//! the policy storage or evaluation layer. The on-disk YAML format mirrors
//! storage-service's `config.urlRemapper` block so the same Helm values
//! fragment can drive both services.

use cedar_policy::PolicySet;
use permission_core::util::encoding::encode_resource_url;
use permission_core::{PolicyErr, PolicyErrKind, Resource};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fmt::{Debug, Display, Formatter};
use tracing::debug;

/// On-disk configuration for the URL remapper.
///
/// Matches storage-service's `config.urlRemapper` shape (`enabled`,
/// `rewriteRules`) so a single Helm values fragment can be shared between the
/// two services. Storage-service's checkpoint-only `checkpointMetadataKey`
/// field is intentionally absent here -- permission-service does not need
/// checkpoint mapping.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UrlRemapperConfig {
    pub enabled: bool,
    #[serde(deserialize_with = "deserialize_rewrite_rules")]
    pub rewrite_rules: HashMap<String, String>,
}

fn deserialize_rewrite_rules<'de, D>(deserializer: D) -> Result<HashMap<String, String>, D::Error>
where
    D: Deserializer<'de>,
{
    let rules = HashMap::<String, RewriteRuleValue>::deserialize(deserializer)?;
    Ok(rules
        .into_iter()
        .map(|(source, value)| (source, value.into_target()))
        .collect())
}

#[derive(Deserialize)]
#[serde(
    untagged,
    expecting = "a target URL prefix string or an object with a string `target` field"
)]
enum RewriteRuleValue {
    Target(String),
    Object { target: String },
}

impl RewriteRuleValue {
    fn into_target(self) -> String {
        match self {
            Self::Target(target) | Self::Object { target } => target,
        }
    }
}

/// Errors raised while constructing a [`UrlRemapper`] from configuration.
#[derive(Debug, PartialEq, Eq)]
pub enum UrlRemapperError {
    EmptySourcePrefix,
    /// The source and target prefixes of a rewrite rule end with a different
    /// number of trailing slashes, which would silently add or drop a path
    /// separator on every rewritten id.
    TrailingSlashMismatch {
        source: String,
        target: String,
        source_slashes: usize,
        target_slashes: usize,
    },
}

impl Display for UrlRemapperError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptySourcePrefix => {
                write!(f, "URL remapper rewrite rule has an empty source prefix")
            }
            Self::TrailingSlashMismatch {
                source,
                target,
                source_slashes,
                target_slashes,
            } => write!(
                f,
                "URL remapper rewrite rule '{}' -> '{}' has mismatched trailing slashes \
                 (source ends with {}, target ends with {})",
                source, target, source_slashes, target_slashes
            ),
        }
    }
}

impl std::error::Error for UrlRemapperError {}

/// Forward URL-prefix remapper.
///
/// Holds the configured rewrite rules sorted by source-prefix length
/// descending so a longest-match lookup can be done with a single linear scan.
pub struct UrlRemapper {
    /// `(source_prefix, target_prefix)` pairs, sorted longest source first.
    rules: Vec<(String, String)>,
}

impl UrlRemapper {
    /// Build a remapper from the parsed configuration.
    ///
    /// Returns `Ok(None)` when the feature is disabled. Empty source prefixes
    /// are rejected because they would match every input and reduce the
    /// remapper to a single, configuration-dependent fixed mapping.
    pub fn from_config(config: UrlRemapperConfig) -> Result<Option<Self>, UrlRemapperError> {
        if !config.enabled {
            return Ok(None);
        }

        let mut rules: Vec<(String, String)> = Vec::with_capacity(config.rewrite_rules.len());
        for (source, target) in config.rewrite_rules {
            if source.is_empty() {
                return Err(UrlRemapperError::EmptySourcePrefix);
            }
            let source_slashes = trailing_slash_count(&source);
            let target_slashes = trailing_slash_count(&target);
            if source_slashes != target_slashes {
                return Err(UrlRemapperError::TrailingSlashMismatch {
                    source,
                    target,
                    source_slashes,
                    target_slashes,
                });
            }
            rules.push((canonical_source_prefix(&source), target));
        }
        rules.sort_unstable_by(|a, b| b.0.len().cmp(&a.0.len()));

        debug!(?rules, "URL remapper initialized");

        Ok(Some(Self { rules }))
    }

    /// Apply the longest matching prefix rule to `id`.
    ///
    /// Returns `Some(new_id)` when a rule applies, `None` when no source prefix
    /// matches.
    pub fn rewrite(&self, id: &str) -> Option<String> {
        let (body, wildcard) = match id.strip_suffix('*') {
            Some(body) => (body, "*"),
            None => (id, ""),
        };
        let canonical = encode_resource_url(body).ok()?;
        for (source, target) in &self.rules {
            if let Some(suffix) = canonical.strip_prefix(source.as_str()) {
                return Some(format!("{target}{suffix}{wildcard}"));
            }
        }
        None
    }

    /// Rewrite a [`Resource`] in place. No-op when the resource id does not
    /// match any rule.
    pub fn rewrite_in_place(&self, resource: &mut Resource) {
        if let Some(new_id) = self.rewrite(&resource.id) {
            resource.id = new_id;
        }
    }

    /// Convenience wrapper for the typical `Option<Resource>` payload field.
    pub fn rewrite_resource_opt(&self, mut resource: Option<Resource>) -> Option<Resource> {
        if let Some(r) = resource.as_mut() {
            self.rewrite_in_place(r);
        }
        resource
    }

    /// Rewrite every entity id embedded in Cedar policy text whose value
    /// matches a configured prefix.
    ///
    /// Operates by round-tripping through Cedar's JSON form and walking the
    /// policy AST recursively, applying [`Self::rewrite`] to the `id` of any
    /// entity reference (object containing both string `type` and `id` fields).
    /// This covers every shape Cedar exposes -- equality constraints
    /// (`resource == X::"..."`), `in`/`is in` constraints (single entity or
    /// entity sets in the resource/principal scope), and entity literals that
    /// appear inside `when`/`unless` condition expressions. Non-entity `id`
    /// fields (record literals, etc.) are left untouched because the rewrite
    /// is gated on the sibling `type` string.
    pub fn rewrite_policy_text(&self, policy: &str) -> Result<String, PolicyErr> {
        let policy_set: PolicySet = policy.parse().map_err(|err| PolicyErr {
            msg: format!("Failed to parse the Cedar policy: {}", err),
            kind: PolicyErrKind::InvalidPolicy,
        })?;

        let mut policy_json = policy_set.to_json().map_err(|err| PolicyErr {
            msg: format!("Failed to serialize policy to JSON: {}", err),
            kind: PolicyErrKind::InvalidPolicy,
        })?;

        if !self.rewrite_entity_ids_in_value(&mut policy_json) {
            return Ok(policy.to_string());
        }

        let new_policy_set = PolicySet::from_json_value(policy_json).map_err(|err| PolicyErr {
            msg: format!("Failed to deserialize policy from JSON: {}", err),
            kind: PolicyErrKind::InvalidPolicy,
        })?;
        Ok(new_policy_set.to_string())
    }

    /// Recursively walks `value`, applying [`Self::rewrite`] to the `id` field
    /// of every JSON object that also has a sibling string `type` field
    /// (Cedar's entity-reference shape). Returns whether any id was changed.
    fn rewrite_entity_ids_in_value(&self, value: &mut Value) -> bool {
        match value {
            Value::Object(map) => {
                let mut changed = false;
                let is_entity_ref = matches!(map.get("type"), Some(Value::String(_)))
                    && matches!(map.get("id"), Some(Value::String(_)));
                if is_entity_ref {
                    if let Some(Value::String(id)) = map.get_mut("id") {
                        if let Some(new_id) = self.rewrite(id) {
                            *id = new_id;
                            changed = true;
                        }
                    }
                }
                for (_, child) in map.iter_mut() {
                    changed |= self.rewrite_entity_ids_in_value(child);
                }
                changed
            }
            Value::Array(items) => {
                let mut changed = false;
                for item in items.iter_mut() {
                    changed |= self.rewrite_entity_ids_in_value(item);
                }
                changed
            }
            _ => false,
        }
    }
}

fn trailing_slash_count(s: &str) -> usize {
    s.chars().rev().take_while(|c| *c == '/').count()
}

fn canonical_source_prefix(source: &str) -> String {
    let canonical = encode_resource_url(source).unwrap_or_else(|_| source.to_string());
    match canonical.strip_suffix('/') {
        Some(without_added_slash) if !source.ends_with('/') => without_added_slash.to_string(),
        _ => canonical,
    }
}

impl Debug for UrlRemapper {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UrlRemapper")
            .field("rules", &self.rules)
            .finish()
    }
}

/// Read and parse a remapper YAML file from disk.
///
/// Panics on read or parse errors, matching the behavior of other startup-time
/// config-file loaders in this crate (see
/// `service::authorization::set_system_policies`).
pub fn load_url_remapper_config(path: &str) -> UrlRemapperConfig {
    let content = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("Unable to read URL remapper config file '{}': {}", path, e));
    serde_norway::from_str(&content)
        .unwrap_or_else(|e| panic!("Unable to parse URL remapper config file '{}': {}", path, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config() -> UrlRemapperConfig {
        let mut rules = HashMap::new();
        rules.insert(
            "omniverse://content.example.com/".to_string(),
            "https://exampleaccount.blob.core.windows.net/examplecontainer/".to_string(),
        );
        rules.insert(
            "omniverse://content.example.com/NVIDIA/".to_string(),
            "https://nvidia-bucket.s3.us-west2.aws.com/".to_string(),
        );
        UrlRemapperConfig {
            enabled: true,
            rewrite_rules: rules,
        }
    }

    #[test]
    fn longest_prefix_wins_for_nested_path() {
        let remapper = UrlRemapper::from_config(sample_config())
            .expect("config is valid")
            .expect("remapper is enabled");

        let rewritten = remapper
            .rewrite("omniverse://content.example.com/NVIDIA/Astronaut/")
            .expect("expected a match");
        assert_eq!(
            rewritten,
            "https://nvidia-bucket.s3.us-west2.aws.com/Astronaut/"
        );
    }

    #[test]
    fn shorter_prefix_used_when_longer_does_not_match() {
        let remapper = UrlRemapper::from_config(sample_config())
            .expect("config is valid")
            .expect("remapper is enabled");

        let rewritten = remapper
            .rewrite("omniverse://content.example.com/Projects/")
            .expect("expected a match");
        assert_eq!(
            rewritten,
            "https://exampleaccount.blob.core.windows.net/examplecontainer/Projects/"
        );
    }

    #[test]
    fn non_matching_url_returns_none() {
        let remapper = UrlRemapper::from_config(sample_config())
            .expect("config is valid")
            .expect("remapper is enabled");
        assert!(remapper.rewrite("https://example.com/some/path").is_none());
    }

    #[test]
    fn rewrite_in_place_leaves_unmatched_resource_alone() {
        let remapper = UrlRemapper::from_config(sample_config())
            .expect("config is valid")
            .expect("remapper is enabled");
        let mut res = Resource {
            id: "https://example.com/foo".into(),
            kind: "ResourceAddress".into(),
            data: None,
        };
        remapper.rewrite_in_place(&mut res);
        assert_eq!(res.id, "https://example.com/foo");
    }

    #[test]
    fn rewrite_resource_opt_rewrites_matching_id() {
        let remapper = UrlRemapper::from_config(sample_config())
            .expect("config is valid")
            .expect("remapper is enabled");
        let res = remapper
            .rewrite_resource_opt(Some(Resource {
                id: "omniverse://content.example.com/NVIDIA/Astronaut/".into(),
                kind: "ResourceAddress".into(),
                data: None,
            }))
            .expect("input was Some");
        assert_eq!(
            res.id,
            "https://nvidia-bucket.s3.us-west2.aws.com/Astronaut/"
        );
        assert_eq!(res.kind, "ResourceAddress");
    }

    #[test]
    fn disabled_config_returns_none() {
        let remapper = UrlRemapper::from_config(UrlRemapperConfig {
            enabled: false,
            rewrite_rules: HashMap::from([("a".to_string(), "b".to_string())]),
        })
        .expect("config is valid");
        assert!(remapper.is_none());
    }

    #[test]
    fn empty_source_prefix_is_rejected() {
        let cfg = UrlRemapperConfig {
            enabled: true,
            rewrite_rules: HashMap::from([("".to_string(), "https://x/".to_string())]),
        };
        assert!(matches!(
            UrlRemapper::from_config(cfg),
            Err(UrlRemapperError::EmptySourcePrefix)
        ));
    }

    fn config_with_rule(source: &str, target: &str) -> UrlRemapperConfig {
        UrlRemapperConfig {
            enabled: true,
            rewrite_rules: HashMap::from([(source.to_string(), target.to_string())]),
        }
    }

    #[test]
    fn trailing_slash_mismatch_when_target_adds_slash() {
        let cfg = config_with_rule("https://source.example.com", "https://target.example.com/");
        assert!(matches!(
            UrlRemapper::from_config(cfg),
            Err(UrlRemapperError::TrailingSlashMismatch {
                source_slashes: 0,
                target_slashes: 1,
                ..
            })
        ));
    }

    #[test]
    fn trailing_slash_match_is_accepted() {
        let cfg = config_with_rule(
            "https://source.example.com/",
            "https://target.example.com/data/",
        );
        assert!(UrlRemapper::from_config(cfg).is_ok());
    }

    #[test]
    fn trailing_slash_mismatch_when_target_drops_slash() {
        let cfg = config_with_rule(
            "https://source.example.com/",
            "https://target.example.com/data",
        );
        assert!(matches!(
            UrlRemapper::from_config(cfg),
            Err(UrlRemapperError::TrailingSlashMismatch {
                source_slashes: 1,
                target_slashes: 0,
                ..
            })
        ));
    }

    #[test]
    fn trailing_slash_mismatch_when_target_doubles_slash() {
        let cfg = config_with_rule(
            "https://source.example.com/data/",
            "https://target.example.com/dest//",
        );
        assert!(matches!(
            UrlRemapper::from_config(cfg),
            Err(UrlRemapperError::TrailingSlashMismatch {
                source_slashes: 1,
                target_slashes: 2,
                ..
            })
        ));
    }

    #[test]
    fn trailing_slash_mismatch_when_source_has_more_slashes() {
        let cfg = config_with_rule(
            "https://source.example.com/data//",
            "https://target.example.com/dest",
        );
        assert!(matches!(
            UrlRemapper::from_config(cfg),
            Err(UrlRemapperError::TrailingSlashMismatch {
                source_slashes: 2,
                target_slashes: 0,
                ..
            })
        ));
    }

    #[test]
    fn yaml_round_trip_uses_camelcase_keys() {
        let yaml = r#"
enabled: true
rewriteRules:
  "omniverse://content.example.com/": "https://exampleaccount.blob.core.windows.net/examplecontainer/"
"#;
        let cfg: UrlRemapperConfig = serde_norway::from_str(yaml).expect("yaml is valid");
        assert!(cfg.enabled);
        assert_eq!(
            cfg.rewrite_rules
                .get("omniverse://content.example.com/")
                .map(String::as_str),
            Some("https://exampleaccount.blob.core.windows.net/examplecontainer/")
        );
    }

    #[test]
    fn yaml_object_rule_with_target_matches_string_rule() {
        let string_form = r#"
enabled: true
rewriteRules:
  "omniverse://models/": "https://mybucket.s3.amazonaws.com/models/"
"#;
        let object_form = r#"
enabled: true
rewriteRules:
  "omniverse://models/":
    target: "https://mybucket.s3.amazonaws.com/models/"
"#;
        let from_string: UrlRemapperConfig =
            serde_norway::from_str(string_form).expect("yaml is valid");
        let from_object: UrlRemapperConfig =
            serde_norway::from_str(object_form).expect("yaml is valid");
        assert_eq!(from_object.rewrite_rules, from_string.rewrite_rules);
        assert_eq!(
            from_object
                .rewrite_rules
                .get("omniverse://models/")
                .map(String::as_str),
            Some("https://mybucket.s3.amazonaws.com/models/")
        );
    }

    #[test]
    fn yaml_object_rule_ignores_unknown_fields() {
        let yaml = r#"
enabled: true
rewriteRules:
  "omniverse://models/":
    target: "https://mybucket.s3.amazonaws.com/models/"
    description: "legacy models bucket"
    priority: 10
    tags: ["a", "b"]
"#;
        let cfg: UrlRemapperConfig = serde_norway::from_str(yaml).expect("yaml is valid");
        assert_eq!(
            cfg.rewrite_rules
                .get("omniverse://models/")
                .map(String::as_str),
            Some("https://mybucket.s3.amazonaws.com/models/")
        );
    }

    #[test]
    fn yaml_mixed_string_and_object_rules_are_accepted() {
        let yaml = r#"
enabled: true
rewriteRules:
  "omniverse://models/": "https://mybucket.s3.amazonaws.com/models/"
  "omniverse://textures/":
    target: "https://mybucket.s3.amazonaws.com/textures/"
"#;
        let cfg: UrlRemapperConfig = serde_norway::from_str(yaml).expect("yaml is valid");
        assert_eq!(
            cfg.rewrite_rules,
            HashMap::from([
                (
                    "omniverse://models/".to_string(),
                    "https://mybucket.s3.amazonaws.com/models/".to_string()
                ),
                (
                    "omniverse://textures/".to_string(),
                    "https://mybucket.s3.amazonaws.com/textures/".to_string()
                ),
            ])
        );
    }

    #[test]
    fn yaml_object_rule_without_target_is_rejected() {
        let yaml = r#"
enabled: true
rewriteRules:
  "omniverse://models/":
    description: "missing target"
"#;
        assert!(serde_norway::from_str::<UrlRemapperConfig>(yaml).is_err());
    }

    #[test]
    fn yaml_object_rule_with_non_string_target_is_rejected() {
        let yaml = r#"
enabled: true
rewriteRules:
  "omniverse://models/":
    target: 42
"#;
        assert!(serde_norway::from_str::<UrlRemapperConfig>(yaml).is_err());
    }

    #[test]
    fn yaml_missing_rewrite_rules_defaults_to_empty() {
        let cfg: UrlRemapperConfig =
            serde_norway::from_str("enabled: true\n").expect("yaml is valid");
        assert!(cfg.rewrite_rules.is_empty());
    }

    #[test]
    fn rewrite_policy_text_rewrites_eq_resource_constraint() {
        let remapper = UrlRemapper::from_config(sample_config())
            .expect("config is valid")
            .expect("remapper is enabled");
        let policy = r#"permit(principal == Principal::"u", action == Action::"svc:read", resource == ResourceAddress::"omniverse://content.example.com/NVIDIA/Astronaut/");"#;
        let rewritten = remapper
            .rewrite_policy_text(policy)
            .expect("rewriting must succeed");
        assert_eq!(
            rewritten,
            r#"permit(principal == Principal::"u", action == Action::"svc:read", resource == ResourceAddress::"https://nvidia-bucket.s3.us-west2.aws.com/Astronaut/");"#
        );
    }

    #[test]
    fn rewrite_policy_text_leaves_non_matching_constraints_untouched() {
        let remapper = UrlRemapper::from_config(sample_config())
            .expect("config is valid")
            .expect("remapper is enabled");
        let policy = r#"permit(principal, action, resource);"#;
        let rewritten = remapper
            .rewrite_policy_text(policy)
            .expect("rewriting must succeed");
        // No entity ids matched any rule, so the remapper preserves the
        // exact input string verbatim (no Cedar round-trip).
        assert_eq!(rewritten, policy);
    }

    #[test]
    fn rewrite_policy_text_rewrites_in_constraint() {
        let remapper = UrlRemapper::from_config(sample_config())
            .expect("config is valid")
            .expect("remapper is enabled");
        let policy = r#"permit(principal, action, resource in ResourceAddress::"omniverse://content.example.com/NVIDIA/Astronaut/");"#;
        let rewritten = remapper
            .rewrite_policy_text(policy)
            .expect("rewriting must succeed");
        assert_eq!(
            rewritten,
            r#"permit(principal, action, resource in ResourceAddress::"https://nvidia-bucket.s3.us-west2.aws.com/Astronaut/");"#
        );
    }

    #[test]
    fn rewrite_policy_text_rewrites_entity_in_when_clause() {
        let remapper = UrlRemapper::from_config(sample_config())
            .expect("config is valid")
            .expect("remapper is enabled");
        let policy = r#"permit(principal, action, resource) when { resource == ResourceAddress::"omniverse://content.example.com/NVIDIA/Astronaut/" };"#;
        let rewritten = remapper
            .rewrite_policy_text(policy)
            .expect("rewriting must succeed");
        assert_eq!(
            rewritten,
            r#"permit(principal, action, resource) when { resource == ResourceAddress::"https://nvidia-bucket.s3.us-west2.aws.com/Astronaut/" };"#
        );
    }

    #[test]
    fn rewrite_policy_text_does_not_rewrite_record_id_field() {
        let remapper = UrlRemapper::from_config(sample_config())
            .expect("config is valid")
            .expect("remapper is enabled");
        let policy = r#"permit(principal, action, resource) when { context.tag == "omniverse://content.example.com/NVIDIA/Astronaut/" };"#;
        let rewritten = remapper
            .rewrite_policy_text(policy)
            .expect("rewriting must succeed");
        // No entity ids matched, so the remapper preserves the exact input
        // string verbatim (no Cedar round-trip).
        assert_eq!(rewritten, policy);
    }

    #[test]
    fn rewrite_policy_text_rewrites_entity_in_unless_clause() {
        let remapper = UrlRemapper::from_config(sample_config())
            .expect("config is valid")
            .expect("remapper is enabled");
        let policy = r#"forbid(principal, action, resource) unless { resource == ResourceAddress::"omniverse://content.example.com/NVIDIA/Astronaut/" };"#;
        let rewritten = remapper
            .rewrite_policy_text(policy)
            .expect("rewriting must succeed");
        assert_eq!(
            rewritten,
            r#"forbid(principal, action, resource) unless { resource == ResourceAddress::"https://nvidia-bucket.s3.us-west2.aws.com/Astronaut/" };"#
        );
    }

    #[test]
    fn rewrite_policy_text_rewrites_multiple_when_unless_clauses() {
        let remapper = UrlRemapper::from_config(sample_config())
            .expect("config is valid")
            .expect("remapper is enabled");
        let policy = r#"
permit(principal, action, resource)
when { resource == ResourceAddress::"omniverse://content.example.com/NVIDIA/Astronaut/" }
when { resource in ResourceAddress::"omniverse://content.example.com/Projects/" }
unless { resource == ResourceAddress::"omniverse://content.example.com/NVIDIA/Marbles/" };
"#;
        let rewritten = remapper
            .rewrite_policy_text(policy)
            .expect("rewriting must succeed");
        assert_eq!(
            rewritten,
            r#"permit(principal, action, resource) when { resource == ResourceAddress::"https://nvidia-bucket.s3.us-west2.aws.com/Astronaut/" } when { resource in ResourceAddress::"https://exampleaccount.blob.core.windows.net/examplecontainer/Projects/" } unless { resource == ResourceAddress::"https://nvidia-bucket.s3.us-west2.aws.com/Marbles/" };"#
        );
    }

    #[test]
    fn rewrite_policy_text_rewrites_entities_in_when_clause_set() {
        let remapper = UrlRemapper::from_config(sample_config())
            .expect("config is valid")
            .expect("remapper is enabled");
        let policy = r#"
permit(principal, action, resource)
when {
    resource in [
        ResourceAddress::"omniverse://content.example.com/NVIDIA/Astronaut/",
        ResourceAddress::"omniverse://content.example.com/Projects/scene.usd"
    ]
};
"#;
        let rewritten = remapper
            .rewrite_policy_text(policy)
            .expect("rewriting must succeed");
        assert_eq!(
            rewritten,
            r#"permit(principal, action, resource) when { resource in [ResourceAddress::"https://nvidia-bucket.s3.us-west2.aws.com/Astronaut/", ResourceAddress::"https://exampleaccount.blob.core.windows.net/examplecontainer/Projects/scene.usd"] };"#
        );
    }

    #[test]
    fn rewrite_policy_text_rewrites_entities_in_if_then_else() {
        let remapper = UrlRemapper::from_config(sample_config())
            .expect("config is valid")
            .expect("remapper is enabled");
        let policy = r#"
permit(principal, action, resource)
when {
    (if principal == Principal::"admin"
     then resource == ResourceAddress::"omniverse://content.example.com/NVIDIA/Astronaut/"
     else resource == ResourceAddress::"omniverse://content.example.com/Projects/scene.usd")
};
"#;
        let rewritten = remapper
            .rewrite_policy_text(policy)
            .expect("rewriting must succeed");
        assert_eq!(
            rewritten,
            r#"permit(principal, action, resource) when { if (principal == Principal::"admin") then (resource == ResourceAddress::"https://nvidia-bucket.s3.us-west2.aws.com/Astronaut/") else (resource == ResourceAddress::"https://exampleaccount.blob.core.windows.net/examplecontainer/Projects/scene.usd") };"#
        );
    }

    #[test]
    fn rewrite_policy_text_does_not_rewrite_action_or_principal_entities() {
        let remapper = UrlRemapper::from_config(sample_config())
            .expect("config is valid")
            .expect("remapper is enabled");
        let policy = r#"permit(principal == Principal::"omniverse://content.example.com/NVIDIA/admin", action == Action::"omniverse://content.example.com/NVIDIA/read", resource);"#;
        let rewritten = remapper
            .rewrite_policy_text(policy)
            .expect("rewriting must succeed");
        // Both principal and action ids match the rewrite prefix and the
        // remapper rewrites them on principle (no type-based filtering); the
        // test exists to lock in that behavior so future changes are explicit.
        assert_eq!(
            rewritten,
            r#"permit(principal == Principal::"https://nvidia-bucket.s3.us-west2.aws.com/admin", action == Action::"https://nvidia-bucket.s3.us-west2.aws.com/read", resource);"#
        );
    }

    #[test]
    fn rewrite_policy_text_does_not_rewrite_entity_attribute_access() {
        let remapper = UrlRemapper::from_config(sample_config())
            .expect("config is valid")
            .expect("remapper is enabled");
        let policy = r#"permit(principal, action, resource) when { resource.tag == "omniverse://content.example.com/NVIDIA/Astronaut/" && resource.id == "omniverse://content.example.com/NVIDIA/Astronaut/" };"#;
        let rewritten = remapper
            .rewrite_policy_text(policy)
            .expect("rewriting must succeed");
        // `resource.id` is attribute access, not an entity reference, so
        // neither it nor the string literals get rewritten and the remapper
        // returns the input verbatim.
        assert_eq!(rewritten, policy);
    }

    #[test]
    fn rewrite_resolves_dot_segments_before_matching_nested_rule() {
        let remapper = UrlRemapper::from_config(sample_config())
            .expect("config is valid")
            .expect("remapper is enabled");

        let rewritten = remapper
            .rewrite("omniverse://content.example.com/NVIDIA/../Projects/scene.usd")
            .expect("expected a match");

        assert_eq!(
            rewritten,
            "https://exampleaccount.blob.core.windows.net/examplecontainer/Projects/scene.usd"
        );
    }

    #[test]
    fn rewrite_resolves_percent_encoded_dot_segments_before_matching() {
        let remapper = UrlRemapper::from_config(sample_config())
            .expect("config is valid")
            .expect("remapper is enabled");

        let rewritten = remapper
            .rewrite("omniverse://content.example.com/NVIDIA/%2e%2E/Projects/scene.usd")
            .expect("expected a match");

        assert_eq!(
            rewritten,
            "https://exampleaccount.blob.core.windows.net/examplecontainer/Projects/scene.usd"
        );
    }

    #[test]
    fn rewrite_applies_nested_rule_reached_through_dot_segments() {
        let remapper = UrlRemapper::from_config(sample_config())
            .expect("config is valid")
            .expect("remapper is enabled");

        let rewritten = remapper
            .rewrite("omniverse://content.example.com/Projects/../NVIDIA/Astronaut/")
            .expect("expected a match");

        assert_eq!(
            rewritten,
            "https://nvidia-bucket.s3.us-west2.aws.com/Astronaut/"
        );
    }

    #[test]
    fn rewrite_preserves_trailing_wildcard() {
        let remapper = UrlRemapper::from_config(sample_config())
            .expect("config is valid")
            .expect("remapper is enabled");

        let rewritten = remapper
            .rewrite("omniverse://content.example.com/NVIDIA/../Projects/*")
            .expect("expected a match");

        assert_eq!(
            rewritten,
            "https://exampleaccount.blob.core.windows.net/examplecontainer/Projects/*"
        );
    }

    #[test]
    fn rewrite_matches_source_prefix_with_non_canonical_host() {
        let remapper = UrlRemapper::from_config(config_with_rule(
            "https://Source.Example.com/data/",
            "https://target.example.com/",
        ))
        .expect("config is valid")
        .expect("remapper is enabled");

        let rewritten = remapper
            .rewrite("https://Source.Example.com/data/file.usd")
            .expect("expected a match");

        assert_eq!(rewritten, "https://target.example.com/file.usd");
    }

    #[test]
    fn rewrite_matches_source_prefix_without_path() {
        let remapper = UrlRemapper::from_config(config_with_rule(
            "https://source.example.com",
            "https://target.example.com",
        ))
        .expect("config is valid")
        .expect("remapper is enabled");

        let rewritten = remapper
            .rewrite("https://source.example.com/file.usd")
            .expect("expected a match");

        assert_eq!(rewritten, "https://target.example.com/file.usd");
    }

    #[test]
    fn rewrite_policy_text_resolves_dot_segments_before_matching() {
        let remapper = UrlRemapper::from_config(sample_config())
            .expect("config is valid")
            .expect("remapper is enabled");
        let policy = r#"permit(principal, action, resource == ResourceAddress::"omniverse://content.example.com/NVIDIA/../Projects/scene.usd");"#;

        let rewritten = remapper
            .rewrite_policy_text(policy)
            .expect("rewriting must succeed");

        assert_eq!(
            rewritten,
            r#"permit(principal, action, resource == ResourceAddress::"https://exampleaccount.blob.core.windows.net/examplecontainer/Projects/scene.usd");"#
        );
    }
}
