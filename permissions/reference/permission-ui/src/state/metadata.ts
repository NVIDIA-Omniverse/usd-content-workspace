/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
import type { User } from "oidc-client-ts";
import z from "zod";
import type { Config } from "../context/ConfigContext";

export interface FetchServiceMetaParams {
  service: string;
  config: Config;
  user: User;
}

export async function fetchServiceMeta({
  service,
  config,
  user,
}: FetchServiceMetaParams): Promise<ServiceMeta> {
  const url = new URL(`/v1beta/services/${service}/`, config.endpoints.backend);

  const response = await fetch(url, {
    headers: {
      Authorization: `Bearer ${user.access_token}`,
    },
  });
  if (response.ok) {
    const json = await response.json();
    return ServiceMeta.parse(json);
  }

  const text = await response.text();
  throw new Error(
    `Failed to load service metadata: HTTP${response.status} -- ${text}`,
  );
}

const ServiceMeta = z.object({
  service: z.string(),
  id_claim: z.string(),
});

export type ServiceMeta = z.infer<typeof ServiceMeta>;

export interface FetchServicesParams {
  config: Config;
  user: User;
}

export async function fetchServices({
  config,
  user,
}: FetchServicesParams): Promise<ServiceMeta[]> {
  const url = new URL("/v1beta/services/", config.endpoints.backend);

  const response = await fetch(url, {
    headers: {
      Authorization: `Bearer ${user.access_token}`,
    },
  });
  if (response.ok) {
    const json = await response.json();
    return json.map((item: unknown) => ServiceMeta.parse(item));
  }

  const text = await response.text();
  throw new Error(`Failed to load services: HTTP${response.status} -- ${text}`);
}

export interface FetchServiceActionsParams {
  service: string;
  config: Config;
  user: User;
}

export async function fetchServiceActions({
  service,
  config,
  user,
}: FetchServiceActionsParams): Promise<string[]> {
  const url = new URL(
    `/v1beta/services/${service}/actions/`,
    config.endpoints.backend,
  );
  const response = await fetch(url, {
    headers: {
      Authorization: `Bearer ${user.access_token}`,
    },
  });
  if (response.ok) {
    const json = await response.json();
    return json.map((item: unknown) => ActionMeta.parse(item).name);
  }

  const text = await response.text();
  throw new Error(
    `Failed to load service actions: HTTP${response.status} -- ${text}`,
  );
}

const ActionMeta = z.object({
  service: z.string(),
  name: z.string(),
});

export interface FetchServiceResourceTypesParams {
  service: string;
  config: Config;
  user: User;
}

export async function fetchServiceResourceTypes({
  service,
  config,
  user,
}: FetchServiceResourceTypesParams): Promise<ResourceInfo[]> {
  const url = new URL(
    `/v1beta/services/${service}/resource-types/`,
    config.endpoints.backend,
  );
  const response = await fetch(url, {
    headers: {
      Authorization: `Bearer ${user.access_token}`,
    },
  });
  if (response.ok) {
    const json = await response.json();
    return json.map((item: unknown) => {
      const meta = ResourceTypeMeta.parse(item);
      return {
        type: meta.type,
        priority: meta.evaluation_priority,
        parentType: meta.parent_type ?? undefined,
      };
    });
  }

  const text = await response.text();
  throw new Error(
    `Failed to load service resource types: HTTP${response.status} -- ${text}`,
  );
}

const ResourceTypeMeta = z.object({
  service: z.string(),
  type: z.string(),
  evaluation_priority: z.enum(["forbid", "permit"]),
  parent_type: z.string().nullable().optional(),
});

export interface FetchFullServiceInfoParams {
  service: string;
  config: Config;
  user: User;
}

export interface ServiceInfo {
  service: string;
  idClaim: string;
  actions: string[];
  resources: ResourceInfo[];
}

export interface ResourceInfo {
  type: string;
  priority: "permit" | "forbid";
  parentType?: string;
}

export async function fetchFullServiceInfo({
  service,
  config,
  user,
}: FetchFullServiceInfoParams): Promise<ServiceInfo> {
  const [meta, actions, resources] = await Promise.all([
    fetchServiceMeta({ service, config, user }),
    fetchServiceActions({ service, config, user }),
    fetchServiceResourceTypes({ service, config, user }),
  ]);

  return {
    service: meta.service,
    idClaim: meta.id_claim,
    actions,
    resources,
  };
}

export interface SubmitServiceParams {
  values: ServiceInfo;
  config: Config;
  user: User;
}

export async function putService({
  values,
  config,
  user,
}: SubmitServiceParams): Promise<void> {
  await Promise.all([
    putServiceMeta({ values, config, user }),
    putServiceActions({
      service: values.service,
      actions: values.actions,
      config,
      user,
    }),
    putServiceResourceTypes({
      service: values.service,
      resources: values.resources,
      config,
      user,
    }),
  ]);
}

async function putServiceMeta({
  values,
  config,
  user,
}: SubmitServiceParams): Promise<void> {
  const url = new URL(
    `/v1beta/services/${values.service}/`,
    config.endpoints.backend,
  );

  const response = await fetch(url, {
    method: "PUT",
    body: JSON.stringify({
      service: values.service,
      id_claim: values.idClaim,
    }),
    headers: {
      Authorization: `Bearer ${user.access_token}`,
      "Content-Type": "application/json",
    },
  });
  if (!response.ok) {
    const text = await response.text();
    throw new Error(
      `Failed to submit the service: HTTP${response.status} -- ${text}`,
    );
  }
}

interface PutServiceActionsParams {
  service: string;
  actions: string[];
  config: Config;
  user: User;
}

async function putServiceActions({
  service,
  actions,
  config,
  user,
}: PutServiceActionsParams): Promise<void> {
  const url = new URL(
    `/v1beta/services/${service}/actions/`,
    config.endpoints.backend,
  );

  const response = await fetch(url, {
    method: "PUT",
    body: JSON.stringify(
      actions.map((action) => ({
        service,
        name: action,
      })),
    ),
    headers: {
      Authorization: `Bearer ${user.access_token}`,
      "Content-Type": "application/json",
    },
  });
  if (!response.ok) {
    const text = await response.text();
    throw new Error(
      `Failed to submit actions for service ${service}: HTTP${response.status} -- ${text}`,
    );
  }
}

interface PutServiceResourceTypesParams {
  service: string;
  resources: ResourceInfo[];
  config: Config;
  user: User;
}

async function putServiceResourceTypes({
  service,
  resources,
  config,
  user,
}: PutServiceResourceTypesParams): Promise<void> {
  const url = new URL(
    `/v1beta/services/${service}/resource-types/`,
    config.endpoints.backend,
  );

  const response = await fetch(url, {
    method: "PUT",
    body: JSON.stringify(
      resources.map((resource) => ({
        service,
        type: resource.type,
        evaluation_priority: resource.priority,
        ...(resource.parentType ? { parent_type: resource.parentType } : {}),
      })),
    ),
    headers: {
      Authorization: `Bearer ${user.access_token}`,
      "Content-Type": "application/json",
    },
  });
  if (!response.ok) {
    const text = await response.text();
    throw new Error(
      `Failed to submit resource types for ${service} service: HTTP${response.status} -- ${text}`,
    );
  }
}

export interface DeleteServiceParams {
  service: string;
  config: Config;
  user: User;
}

export async function deleteService({
  service,
  config,
  user,
}: DeleteServiceParams): Promise<void> {
  const url = new URL(`/v1beta/services/${service}/`, config.endpoints.backend);
  const response = await fetch(url, {
    method: "DELETE",
    headers: {
      Authorization: `Bearer ${user.access_token}`,
    },
  });
  if (!response.ok) {
    const text = await response.text();
    throw new Error(
      `Failed to delete service ${service}: HTTP${response.status} -- ${text}`,
    );
  }
}
