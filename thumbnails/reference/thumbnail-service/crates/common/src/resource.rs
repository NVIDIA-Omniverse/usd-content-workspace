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

//! Shared `Resource` validation, used by both the HTTP and gRPC transports so the
//! "files have an identity, folders do not" rule has a single definition.

/// Return the canonical URL serialization Storage expects for a resource address.
///
/// Storage events and API callers can supply human-readable paths containing raw
/// spaces or Unicode. `reqwest::Url` accepts those addresses and serializes their
/// path bytes with the required percent-encoding. If an address is not parseable,
/// preserve its trimmed form so the existing validation/downstream error remains
/// authoritative rather than silently changing its meaning.
pub fn canonicalize_resource_address(address: &str) -> String {
    let trimmed = address.trim();
    reqwest::Url::parse(trimmed)
        .map(|url| encode_strict_uri_tail(url.as_ref()))
        .unwrap_or_else(|_| trimmed.to_string())
}

/// Encode printable characters WHATWG URLs allow in paths but RFC 3986 URI paths do not.
/// The authority is excluded so brackets around an IPv6 host remain structural.
fn encode_strict_uri_tail(serialized: &str) -> String {
    let scheme_end = serialized.find(':').map_or(0, |index| index + 1);
    let tail_start = if serialized[scheme_end..].starts_with("//") {
        let authority_start = scheme_end + 2;
        serialized[authority_start..]
            .find(['/', '?', '#'])
            .map_or(serialized.len(), |offset| authority_start + offset)
    } else {
        scheme_end
    };

    let mut canonical = String::with_capacity(serialized.len());
    canonical.push_str(&serialized[..tail_start]);
    for character in serialized[tail_start..].chars() {
        match character {
            '[' => canonical.push_str("%5B"),
            ']' => canonical.push_str("%5D"),
            '^' => canonical.push_str("%5E"),
            '|' => canonical.push_str("%7C"),
            '\\' => canonical.push_str("%5C"),
            _ => canonical.push(character),
        }
    }
    canonical
}

/// Why a resource failed validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceValidationError {
    /// `resource_address` was empty or whitespace.
    MissingAddress,
    /// A file resource was missing its required `resource_identity`.
    MissingIdentity,
    /// A folder resource carried a `resource_identity`, which folders do not have.
    UnexpectedIdentity,
}

impl std::fmt::Display for ResourceValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = match self {
            ResourceValidationError::MissingAddress => "resource_address is required and non-empty",
            ResourceValidationError::MissingIdentity => {
                "resource_identity is required for file resources"
            }
            ResourceValidationError::UnexpectedIdentity => {
                "resource_identity must be omitted for folder resources"
            }
        };
        f.write_str(msg)
    }
}

/// Validate a resource. `require_identity` is true when the resource is a file
/// (thumbnails always; association targets when `target_type` is FILE), false
/// when it is a folder.
pub fn validate_resource(
    resource_address: &str,
    resource_identity: Option<&str>,
    require_identity: bool,
) -> Result<(), ResourceValidationError> {
    if resource_address.trim().is_empty() {
        return Err(ResourceValidationError::MissingAddress);
    }
    let has_identity = resource_identity.is_some_and(|id| !id.trim().is_empty());
    match (require_identity, has_identity) {
        (true, false) => Err(ResourceValidationError::MissingIdentity),
        (false, true) => Err(ResourceValidationError::UnexpectedIdentity),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalization_percent_encodes_raw_path_characters() {
        assert_eq!(
            canonicalize_resource_address(
                "file-storage://fileservice/samples/IMG_2062 (1)-puppyö-[w^x|y\\z].jpg"
            ),
            "file-storage://fileservice/samples/IMG_2062%20(1)-puppyo%CC%88-%5Bw%5Ex%7Cy%5Cz%5D.jpg"
        );
    }

    #[test]
    fn canonicalization_preserves_ipv6_authority_brackets() {
        assert_eq!(
            canonicalize_resource_address("https://[::1]/samples/[image].png"),
            "https://[::1]/samples/%5Bimage%5D.png"
        );
    }

    #[test]
    fn file_requires_identity() {
        assert_eq!(validate_resource("addr", Some("id"), true), Ok(()));
        assert_eq!(
            validate_resource("addr", None, true),
            Err(ResourceValidationError::MissingIdentity)
        );
    }

    #[test]
    fn whitespace_only_identity_treated_as_missing() {
        assert_eq!(
            validate_resource("addr", Some("   "), true),
            Err(ResourceValidationError::MissingIdentity)
        );
    }

    #[test]
    fn folder_rejects_identity() {
        assert_eq!(validate_resource("addr", None, false), Ok(()));
        assert_eq!(
            validate_resource("addr", Some("id"), false),
            Err(ResourceValidationError::UnexpectedIdentity)
        );
    }

    #[test]
    fn blank_address_rejected_first() {
        assert_eq!(
            validate_resource("  ", Some("id"), true),
            Err(ResourceValidationError::MissingAddress)
        );
    }
}
