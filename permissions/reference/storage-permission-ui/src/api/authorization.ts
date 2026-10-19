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
import type { ResourceAddress } from "../types/ResourceAddress";
import { encodeResourceUrl } from "../utils/resources.js";
import { AuthorizationBatchResponseSchema } from "./schemas.js";

export interface CheckUserAccessParams {
  auth: PermissionAuth;
  config: PermissionConfig;
  resource: ResourceAddress;
}

export interface UserAccess {
  canViewPermissions: boolean;
  canEditPermissions: boolean;
}

export async function checkUserAccess(
  params: CheckUserAccessParams,
): Promise<UserAccess> {
  const url = new URL("/v1beta/authorization/batch/", params.config.url);
  const headers = new Headers();
  headers.set("Content-Type", "application/json");

  if (params.auth.accessToken) {
    headers.set("Authorization", `Bearer ${params.auth.accessToken}`);
  }

  const authorizationActions = [
    {
      service: "permissions",
      name: "view",
    },
    {
      service: "permissions",
      name: "edit",
    },
  ];

  const authorizedResource = {
    id: encodeResourceUrl(params.resource.address),
    type: params.resource.type,
  };

  const payload = {
    batches: [
      {
        actions: authorizationActions,
        resource: authorizedResource,
      },
    ],
  };

  const response = await fetch(url, {
    method: "POST",
    headers,
    body: JSON.stringify(payload),
  });

  if (response.ok) {
    const json = await response.json();
    const result = AuthorizationBatchResponseSchema.parse(json);
    const [actions] = result.decisions;
    return {
      canViewPermissions: actions["permissions:view"].decision === "allow",
      canEditPermissions: actions["permissions:edit"].decision === "allow",
    };
  }

  const message = await response.text();
  throw new Error(
    `Failed to retrieve user access -- HTTP${response.status}. ${message}`,
  );
}
