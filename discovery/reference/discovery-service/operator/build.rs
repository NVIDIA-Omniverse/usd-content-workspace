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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    const PROTO_ROOT: &str = "discovery-protos/proto";
    const V2BETA: &str = "discovery-protos/proto/nvidia/omniverse/discovery/capabilities/v2beta";
    // Deprecated; compiled only so tests can exercise the client's v2alpha fallback.
    const V2ALPHA: &str = "discovery-protos/proto/nvidia/omniverse/discovery/capabilities/v2alpha";

    tonic_prost_build::configure().compile_protos(
        &[
            format!("{V2BETA}/capabilities.proto"),
            format!("{V2BETA}/capabilities_service.proto"),
            format!("{V2ALPHA}/capabilities.proto"),
            format!("{V2ALPHA}/capabilities_service.proto"),
        ],
        &[PROTO_ROOT.to_string()],
    )?;
    Ok(())
}
