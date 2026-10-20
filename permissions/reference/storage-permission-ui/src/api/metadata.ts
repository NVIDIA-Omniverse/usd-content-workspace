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

import type { PermissionAuth } from "../types/PermissionAuth";
import type { PermissionConfig } from "../types/PermissionConfig";
import { type ServiceMetadata, ServiceMetadataSchema } from "./schemas.js";

export interface GetMetadataParams {
  auth: PermissionAuth;
  config: PermissionConfig;
}

export async function getMetadata(
  params: GetMetadataParams,
): Promise<ServiceMetadata> {
  const url = new URL("/v1beta/services/storage-service/", params.config.url);
  const headers = new Headers();
  if (params.auth.accessToken) {
    headers.set("Authorization", `Bearer ${params.auth.accessToken}`);
  }

  const response = await fetch(url, {
    headers,
  });

  if (response.ok) {
    const json = await response.json();
    return ServiceMetadataSchema.parse(json);
  }

  const message = await response.text();
  throw new Error(
    `Unable to get storage metadata -- HTTP${response.status}. ${message}`,
  );
}
