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

use thumbnail_service::http::ApiDoc;
use utoipa::openapi::external_docs::ExternalDocsBuilder;
use utoipa::openapi::tag::TagBuilder;
use utoipa::OpenApi;

/// License header for the generated spec, so the committed `thumbnail.openapi.yaml`
/// carries it and the CI drift check keeps it in place.
const LICENSE_HEADER: &str = "\
# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
#
# Licensed under the Apache License, Version 2.0 (the \"License\");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an \"AS IS\" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

";

/// Adds the `nspect_id` tag that NVIDIA's internal API governance pipeline requires.
///
/// The ID and its URL are not part of the source: CI sets `OPENAPI_NSPECT_ID` and
/// `OPENAPI_NSPECT_URL` only when it generates the spec for governance and publishing.
/// Without them the generated spec, like the committed `thumbnail.openapi.yaml`, carries
/// no such tag.
fn add_nspect_tag(openapi: &mut utoipa::openapi::OpenApi) {
    let Ok(id) = std::env::var("OPENAPI_NSPECT_ID") else {
        return;
    };
    let mut tag = TagBuilder::new().name("nspect_id").description(Some(id));
    if let Ok(url) = std::env::var("OPENAPI_NSPECT_URL") {
        tag = tag.external_docs(Some(
            ExternalDocsBuilder::new()
                .url(url)
                .description(Some("nSpect URL"))
                .build(),
        ));
    }
    openapi.tags.get_or_insert_with(Vec::new).push(tag.build());
}

fn main() {
    // Generate the OpenAPI spec from the Rust annotations
    let mut doc = ApiDoc::openapi();
    add_nspect_tag(&mut doc);

    // Convert to JSON and then to YAML for better readability
    let json_spec = serde_json::to_string_pretty(&doc).expect("Failed to serialize OpenAPI spec");

    // Parse JSON and convert to YAML
    let json_value: serde_json::Value =
        serde_json::from_str(&json_spec).expect("Failed to parse JSON");
    let yaml_spec = serde_yaml::to_string(&json_value).expect("Failed to convert to YAML");

    print!("{LICENSE_HEADER}");
    println!("{}", yaml_spec);
}
