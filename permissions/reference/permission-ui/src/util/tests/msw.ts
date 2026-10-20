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
import { http, HttpResponse } from "msw";
import type { DiagnosticsBatchResponse } from "../../state/diagnostics.ts";
import type { ResourceInfo } from "../../state/metadata.ts";
import type { Policy, PolicyPage } from "../../state/policies.ts";
import type {
  GroupInfo,
  GroupPage,
  UserInfo,
  UserPage,
} from "../../state/users.ts";
import { afterAll, afterEach, beforeAll } from "vitest";
import { setupServer } from "msw/node";

export const handlers = [
  http.get("http://localhost:8080/v1beta/policies/", ({ request }) => {
    const url = new URL(request.url);
    const principal = url.searchParams.get("principal");
    const group = url.searchParams.get("group");
    const action = url.searchParams.get("action");
    const order = url.searchParams.get("order");

    let filteredItems = [...mockPolicyPage.items];

    if (principal && principal !== "") {
      filteredItems = filteredItems.filter((policy) =>
        policy.principal?.sub?.includes(principal),
      );
    }

    if (group && group !== "") {
      filteredItems = filteredItems.filter((policy) =>
        policy.group?.id?.includes(group),
      );
    }

    if (action && action !== "NULL" && action !== "") {
      filteredItems = filteredItems.filter(
        (policy) =>
          action.includes(policy.action?.name || "") || !policy.action,
      );
    }

    if (order && order !== "") {
      filteredItems = filteredItems.filter(
        (policy) => policy.order === Number(order),
      );
    }

    return HttpResponse.json({
      ...mockPolicyPage,
      items: filteredItems,
    });
  }),

  http.get("http://localhost:8080/v1beta/policies/error", () => {
    return new HttpResponse("Internal Server Error", { status: 500 });
  }),

  http.get("http://localhost:8080/v1beta/policies/:policyId", ({ params }) => {
    if (params.policyId === "1") {
      return HttpResponse.json(mockPermitPolicy);
    }
    if (params.policyId === "2") {
      return HttpResponse.json(mockForbidPolicy);
    }
    if (params.policyId === "3") {
      return HttpResponse.json(mockGroupPolicy);
    }
    if (params.policyId === "error") {
      return new HttpResponse("Internal Server Error", { status: 500 });
    }
    return new HttpResponse("Not Found", { status: 404 });
  }),

  http.delete(
    "http://localhost:8080/v1beta/policies/:policyId",
    ({ params }) => {
      if (
        params.policyId === "1" ||
        params.policyId === "2" ||
        params.policyId === "3"
      ) {
        return new HttpResponse(null, { status: 204 });
      }
      if (params.policyId === "error") {
        return new HttpResponse("Internal Server Error", { status: 500 });
      }
      return new HttpResponse("Not Found", { status: 404 });
    },
  ),

  http.get("http://localhost:8080/v1beta/services/", () => {
    return HttpResponse.json(mockServices);
  }),

  http.get("http://localhost:8080/v1beta/services/:service", ({ params }) => {
    const service = mockServices.find(
      (service) => params.service === service.service,
    );
    if (service) {
      return HttpResponse.json(service);
    }
    return new HttpResponse(null, { status: 404 });
  }),

  http.get(
    "http://localhost:8080/v1beta/services/:service/actions/",
    ({ params }) => {
      return HttpResponse.json(
        mockServiceActions.map((action) => ({
          service: params.service,
          name: action,
        })),
      );
    },
  ),

  http.get(
    "http://localhost:8080/v1beta/services/:service/resource-types/",
    ({ params }) => {
      return HttpResponse.json(
        mockServiceResourceTypes.map((resource) => ({
          service: params.service,
          type: resource.type,
          evaluation_priority: resource.priority,
          parent_type: resource.parentType ?? null,
        })),
      );
    },
  ),

  http.get("http://localhost:8080/v1beta/services/:service/meta/", () => {
    return HttpResponse.json({ id_claim: "sub" });
  }),

  http.put("http://localhost:8080/v1beta/policies/", () => {
    return HttpResponse.json({
      ...mockPermitPolicy,
      id: "1",
    });
  }),

  http.put("http://localhost:8080/v1beta/services/:service/", () => {
    return new HttpResponse(null, { status: 200 });
  }),

  http.put("http://localhost:8080/v1beta/services/:service/actions/", () => {
    return new HttpResponse(null, { status: 200 });
  }),

  http.put(
    "http://localhost:8080/v1beta/services/:service/resource-types/",
    () => {
      return new HttpResponse(null, { status: 200 });
    },
  ),

  http.delete(
    "http://localhost:8080/v1beta/services/:service",
    ({ params }) => {
      if (params.service === "error-service") {
        return new HttpResponse("Internal Server Error", { status: 500 });
      }
      return new HttpResponse(null, { status: 204 });
    },
  ),

  http.get("http://localhost:9090/api/v1alpha/users", ({ request }) => {
    const url = new URL(request.url);
    const displayName = url.searchParams.get("display_name");
    const email = url.searchParams.get("email");

    let filtered = [...mockUsers];
    if (displayName) {
      const lower = displayName.toLowerCase();
      filtered = filtered.filter((u) =>
        u.display_name?.toLowerCase().includes(lower),
      );
    }
    if (email) {
      const lower = email.toLowerCase();
      filtered = filtered.filter((u) => u.email?.toLowerCase() === lower);
    }

    return HttpResponse.json({
      items: filtered,
      next_cursor: null,
    } satisfies UserPage);
  }),

  http.get("http://localhost:9090/api/v1alpha/groups", ({ request }) => {
    const url = new URL(request.url);
    const displayName = url.searchParams.get("display_name");

    let filtered = [...mockGroups];
    if (displayName) {
      const lower = displayName.toLowerCase();
      filtered = filtered.filter((g) =>
        g.display_name?.toLowerCase().includes(lower),
      );
    }

    return HttpResponse.json({
      items: filtered,
      next_cursor: null,
    } satisfies GroupPage);
  }),

  http.get(
    "http://localhost:9090/api/v1alpha/groups/:groupId",
    ({ params }) => {
      const group = mockGroups.find((g) => g.id === params.groupId);
      if (group) {
        return HttpResponse.json(group);
      }
      return new HttpResponse(
        JSON.stringify({ error: "not_found", message: "Group not found" }),
        { status: 404 },
      );
    },
  ),

  http.get(
    "http://localhost:9090/api/v1alpha/users/:userId/groups",
    ({ params }) => {
      const groups = mockUserGroups[params.userId as string] ?? [];
      return HttpResponse.json({
        items: groups,
        next_cursor: null,
      } satisfies GroupPage);
    },
  ),

  http.get(
    "http://localhost:9090/api/v1alpha/users/:userId",
    ({ params }) => {
      const user = mockUsers.find((u) => u.id === params.userId);
      if (user) {
        return HttpResponse.json(user);
      }
      return new HttpResponse(
        JSON.stringify({ error: "not_found", message: "User not found" }),
        { status: 404 },
      );
    },
  ),

  http.post(
    "http://localhost:8080/v1beta/diagnostics/authorize/batch/",
    () => {
      return HttpResponse.json(mockDiagnosticsBatchResponse);
    },
  ),
];

export const server = setupServer(...handlers);

beforeAll(() => server.listen());
afterEach(() => {
  server.resetHandlers();
});

afterAll(() => server.close());

export const mockPermitPolicy: Policy = {
  id: 1,
  policy:
    'permit(\n  principal == User::"user-123",\n  action == Action::"read",\n  resource == Document::"doc-123"\n);',
  principal: { sub: "user-123", info: null },
  action: { name: "read", service: "document-service" },
  resource: { id: "doc-123", type: "Document", data: null },
  created_at: new Date("2026-01-15T10:30:00Z"),
  created_by: "alice@example.com",
  comment: "owned by platform team",
};

export const mockForbidPolicy: Policy = {
  id: 2,
  policy:
    'forbid(\n  principal,\n  action == Action::"delete",\n  resource == Document::"doc-456"\n);',
  principal: null,
  action: { name: "delete", service: "document-service" },
  resource: { id: "doc-456", type: "Document", data: null },
  created_at: new Date("2026-02-20T14:45:00Z"),
  created_by: "bob@example.com",
  comment: null,
};

export const mockGroupPolicy: Policy = {
  id: 3,
  policy:
    'permit(\n  principal in Group::"group-123",\n  action == Action::"document-service:read",\n  resource\n);',
  principal: null,
  group: { id: "group-123" },
  action: { name: "read", service: "document-service" },
  resource: null,
  created_at: new Date("2026-03-10T09:00:00Z"),
  created_by: "alice@example.com",
  comment: null,
};

export const mockPolicyPage: PolicyPage = {
  items: [mockPermitPolicy, mockForbidPolicy, mockGroupPolicy],
  page: 1,
  page_size: 50,
};

export const mockServices = [
  { service: "document-service", id_claim: "sub" },
  { service: "user-service", id_claim: "sub" },
  { service: "email-service", id_claim: "email" },
];

export const mockServiceActions = ["read", "write", "delete"];

export const mockServiceResourceTypes: ResourceInfo[] = [
  { type: "Document", priority: "permit", parentType: "Folder" },
  { type: "Folder", priority: "forbid" },
];

export const mockDiagnosticsBatchResponse: DiagnosticsBatchResponse = {
  results: [
    {
      resource: { id: "doc-123", type: "Document", data: null },
      evaluation_priority: "permit",
      actions: [
        {
          service: "document-service",
          action: "read",
          policies: [
            {
              id: 1,
              order: 0,
              policy:
                'permit(\n  principal == User::"user-123",\n  action == Action::"document-service:read",\n  resource == Document::"doc-123"\n);',
            },
            {
              id: 2,
              order: 10,
              policy:
                'forbid(\n  principal,\n  action == Action::"document-service:delete",\n  resource == Document::"doc-456"\n);',
            },
          ],
        },
      ],
    },
    {
      resource: { id: "doc-456", type: "Document", data: null },
      evaluation_priority: "forbid",
      actions: [
        {
          service: "document-service",
          action: "read",
          policies: [
            {
              id: 2,
              order: 10,
              policy:
                'forbid(\n  principal,\n  action == Action::"document-service:delete",\n  resource == Document::"doc-456"\n);',
            },
          ],
        },
      ],
    },
  ],
};

export const mockUsers: UserInfo[] = [
  {
    id: "user-123",
    display_name: "Alice Johnson",
    email: "alice@example.com",
    given_name: "Alice",
    family_name: "Johnson",
    username: "alice.johnson",
    active: true,
  },
  {
    id: "user-456",
    display_name: "Bob Smith",
    email: "bob@example.com",
    given_name: "Bob",
    family_name: "Smith",
    username: "bob.smith",
    active: true,
  },
  {
    id: "user-789",
    display_name: "Charlie Brown",
    email: "charlie@example.com",
    given_name: "Charlie",
    family_name: "Brown",
    username: "charlie.brown",
    active: true,
  },
];

export const mockGroups: GroupInfo[] = [
  {
    id: "group-123",
    display_name: "Platform Team",
    description: "Owns platform infrastructure",
  },
  {
    id: "group-456",
    display_name: "Security Team",
    description: "Handles security reviews",
  },
  {
    id: "group-789",
    display_name: "Data Team",
    description: "Manages data pipelines",
  },
];

export const mockUserGroups: Record<string, GroupInfo[]> = {
  "user-123": [mockGroups[0], mockGroups[1]],
};
