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

use permission_service::http::{add_nspect_id, add_public_routes, ApiDoc};
use std::fs::File;
use std::io::Write;
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;

fn main() -> std::io::Result<()> {
    let router = OpenApiRouter::with_openapi(ApiDoc::openapi());
    let router = add_public_routes(router);
    let mut openapi = router.get_openapi().clone();
    // Set only by the CI job that prepares the spec for API governance; see `add_nspect_id`.
    if let Some(id) = std::env::var("OPENAPI_NSPECT_ID")
        .ok()
        .filter(|id| !id.is_empty())
    {
        add_nspect_id(&mut openapi, &id);
    }
    let openapi_json = openapi
        .to_pretty_json()
        .expect("Failed to serialize OpenAPI spec to JSON");

    let mut file = File::create("openapi.json")?;
    file.write_all(openapi_json.as_bytes())?;

    println!("Successfully generated openapi.json");
    Ok(())
}
