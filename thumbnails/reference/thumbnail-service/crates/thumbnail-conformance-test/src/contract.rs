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

//! Independent published-contract expectations for this black-box client.
use serde_json::Value;

use crate::style::{ConfigurationFault, TestError};

pub const OPENAPI_PATH: &str = "/api-docs/openapi.json";
const EXPECTED_MAX_SPECIFICATIONS: u64 = 1000;

/// Constructible only after the deployment's advertised bound matches the contract.
#[derive(Clone, Copy, Debug)]
pub struct VerifiedBound(usize);

impl VerifiedBound {
    pub fn over_bound(self) -> usize {
        self.0 + 1
    }
}

pub fn unverifiable(detail: impl std::fmt::Display) -> TestError {
    TestError::configuration(ConfigurationFault::UnverifiableContract, format!(
        "unverifiable contract at {OPENAPI_PATH}: {detail}; check --rest-url (required even for --test-suite=grpc) and access to the OpenAPI route"
    ))
}

fn resolve<'a>(document: &'a Value, mut schema: &'a Value) -> Result<&'a Value, TestError> {
    // Bound reference traversal so cycles cannot hang a certification run.
    for _ in 0..16 {
        let Some(reference) = schema.get("$ref") else {
            return Ok(schema);
        };
        let pointer = reference
            .as_str()
            .and_then(|s| s.strip_prefix('#'))
            .ok_or_else(|| unverifiable("expected a local schema reference"))?;
        schema = document
            .pointer(pointer)
            .ok_or_else(|| unverifiable(format!("unresolved {reference}")))?;
    }
    Err(unverifiable(
        "cyclic or excessively nested schema references",
    ))
}

pub fn verify_advertised_bound(document: &Value) -> Result<VerifiedBound, TestError> {
    let request = resolve(
        document,
        &document["paths"]["/v1alpha/thumbnails"]["post"]["requestBody"],
    )?;
    let schema = resolve(document, &request["content"]["application/json"]["schema"])?;
    let specifications = resolve(document, &schema["properties"]["specifications"])?;
    let bound = specifications["maxItems"]
        .as_u64()
        .filter(|v| *v > 0)
        .ok_or_else(|| {
            unverifiable("specifications.maxItems is absent or not a positive integer")
        })?;
    if bound != EXPECTED_MAX_SPECIFICATIONS {
        return Err(TestError::new(format!(
            "{OPENAPI_PATH} advertises specifications.maxItems={bound}; documented contract expects {EXPECTED_MAX_SPECIFICATIONS}. No over-bound probe was allocated or sent."
        )));
    }
    Ok(VerifiedBound(bound as usize))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CreateCase {
    Valid,
    Dimensions,
    OverBound,
    BlankAddress,
    JpegQuality,
    WebpQuality,
    PngQuality,
    GifQuality,
}

impl CreateCase {
    pub fn name(self) -> &'static str {
        match self {
            Self::Valid => "valid specification accepted",
            Self::Dimensions => "out-of-range width rejected",
            Self::OverBound => "over-long specification list rejected",
            Self::BlankAddress => "blank resource_address rejected",
            Self::JpegQuality => "JPEG quality 200 rejected",
            Self::WebpQuality => "WebP quality 200 rejected",
            Self::PngQuality => "PNG quality 200 accepted",
            Self::GifQuality => "GIF quality 200 accepted",
        }
    }

    pub fn accepts(self) -> bool {
        matches!(self, Self::Valid | Self::PngQuality | Self::GifQuality)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AssociationRejection {
    BlankAddress,
    MissingSpecification,
}

impl AssociationRejection {
    pub fn name(self) -> &'static str {
        match self {
            Self::BlankAddress => "blank resource_address rejected",
            Self::MissingSpecification => "missing specification rejected",
        }
    }
}

/// Acceptance can start real work; the black-box suite cannot observe publication.
pub fn over_bound_acceptance_consequence(over_bound: usize) -> String {
    format!(
        "The accepted request may generate {over_bound} thumbnails and leave {over_bound} \
         associations on the source. That metadata can exceed storage response limits and break \
         later reads and disassociation for this address. Check what reached the generate queue."
    )
}

#[cfg(test)]
pub(crate) fn document_with_bound(bound: Value) -> Value {
    serde_json::json!({
        "paths": {"/v1alpha/thumbnails": {"post": {"requestBody": {"content": {"application/json": {
            "schema": {"$ref": "#/components/schemas/Create"}
        }}}}}},
        "components": {"schemas": {"Create": {"properties": {"specifications": {"maxItems": bound}}}}}
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifies_independent_bound_before_constructing_a_probe() {
        assert_eq!(
            verify_advertised_bound(&document_with_bound(1000.into()))
                .unwrap()
                .over_bound(),
            1001
        );
        for bound in [1, 999, 1001, u64::MAX] {
            let error = verify_advertised_bound(&document_with_bound(bound.into())).unwrap_err();
            assert!(
                error.message.contains(&format!("maxItems={bound}"))
                    && error.message.contains("expects 1000"),
                "{error}"
            );
        }
        for bound in [
            Value::Null,
            0.into(),
            (-1).into(),
            10.5.into(),
            "1000".into(),
            true.into(),
        ] {
            let error = verify_advertised_bound(&document_with_bound(bound)).unwrap_err();
            assert_eq!(
                error.configuration_fault,
                Some(ConfigurationFault::UnverifiableContract)
            );
            assert!(error.message.contains(OPENAPI_PATH) && error.message.contains("--rest-url"));
        }
        assert!(verify_advertised_bound(&serde_json::json!({})).is_err());
        let mut document = document_with_bound(1000.into());
        document["components"]["schemas"]["Create"] =
            serde_json::json!({"$ref": "#/components/schemas/Create"});
        assert!(verify_advertised_bound(&document)
            .unwrap_err()
            .message
            .contains("cyclic"));
    }

    #[test]
    fn executable_cases_have_transport_counterparts() {
        use std::collections::BTreeSet;
        let rest: BTreeSet<_> = crate::rest::CREATE_CASES.iter().copied().collect();
        let grpc: BTreeSet<_> = crate::grpc::CREATE_CASES.iter().copied().collect();
        if let Some(case) = rest.symmetric_difference(&grpc).next() {
            panic!(
                "missing transport counterpart for CreateThumbnails: {}",
                case.name()
            );
        }
        let rest: BTreeSet<_> = crate::rest::ASSOCIATION_REJECTIONS
            .iter()
            .copied()
            .collect();
        let grpc: BTreeSet<_> = crate::grpc::ASSOCIATION_REJECTIONS
            .iter()
            .copied()
            .collect();
        if let Some(case) = rest.symmetric_difference(&grpc).next() {
            panic!(
                "missing transport counterpart for AssociateThumbnail: {}",
                case.name()
            );
        }
    }
}

/// A quality value such as 200 is not evidence that an error identifies index 0.
pub fn identifies_first_specification_field(message: &str, field: &str) -> bool {
    message.contains(field)
        && ["specification 0", "specifications[0]", "specification[0]"]
            .iter()
            .any(|index| message.contains(index))
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;
    #[test]
    fn invalid_value_does_not_substitute_for_the_specification_index() {
        for message in [
            "quality 200 invalid",
            "specification 1: quality invalid",
            "specification 0: width invalid",
        ] {
            assert!(!identifies_first_specification_field(message, "quality"));
        }
        for message in [
            "specification 0: quality invalid",
            "specifications[0].image_to_image.quality invalid",
        ] {
            assert!(identifies_first_specification_field(message, "quality"));
        }
    }
}
