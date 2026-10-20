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

import z from "zod";

export const PrincipalSchema = z.object({
  sub: z.string(),
  info: z.any().nullable(),
});
export const GroupSchema = z.object({
  id: z.string(),
});
export const ActionSchema = z.object({
  name: z.string(),
  service: z.string(),
});
export const ResourceSchema = z.object({
  id: z.string(),
  type: z.string(),
  data: z.any().nullable(),
});
export const PolicySchema = z.object({
  id: z.number().int(),
  policy: z.string(),
  order: z.number().int().nullish(),
  created_by: z.string().nullish(),
  principal: PrincipalSchema.nullable(),
  group: GroupSchema.nullable().optional(),
  action: ActionSchema.nullable(),
  resource: ResourceSchema.nullable(),
  resource_prefix: ResourceSchema.nullable().optional(),
});
export type Policy = z.infer<typeof PolicySchema>;
export const PolicyPageSchema = z.object({
  items: z.array(PolicySchema),
  page: z.number(),
  page_size: z.number(),
});
export type PolicyPage = z.infer<typeof PolicyPageSchema>;

export const ServiceMetadataSchema = z.object({
  service: z.string(),
  id_claim: z.string(),
});
export type ServiceMetadata = z.infer<typeof ServiceMetadataSchema>;

export const AuthorizationResultSchema = z.object({
  decision: z.enum(["allow", "deny"]),
  reason: z.string().optional(),
});

export const AuthorizationBatchResponseItemSchema = z.record(
  z.string(),
  AuthorizationResultSchema,
);

export const AuthorizationBatchResponseSchema = z.object({
  decisions: z.array(AuthorizationBatchResponseItemSchema),
});
