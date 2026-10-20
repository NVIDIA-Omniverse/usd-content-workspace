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

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { setupServer } from "msw/node";
import { http, HttpResponse } from "msw";
import {
  ALL_USERS,
  getGlobalPolicies,
  getPermissions,
  groupKey,
  submitPermissions,
  type GetGlobalPoliciesParams,
  type GetPermissionsParams,
  type SubmitPermissionsParams,
  type PolicyPayload,
} from "./policies.js";
import type { PermissionAuth } from "../types/PermissionAuth";
import type { PermissionConfig } from "../types/PermissionConfig";
import type { ResourceAddress } from "../types/ResourceAddress";
import type { PolicyPage, Policy } from "./schemas";

const mockAuth: PermissionAuth = {
  accessToken: "test-access-token",
  idToken: "test-id-token",
  onExpire: vi.fn(),
};

const mockConfig: PermissionConfig = {
  url: "https://api.example.com",
};

const mockResource: ResourceAddress = {
  address: "https://example.com/folder/test",
  type: "folder",
};

const mockPolicyPage: PolicyPage = {
  items: [
    {
      id: 1,
      policy:
        'permit(principal == Principal::"user1", action == Action::"storage-service:read", resource == folder::"https://example.com/folder/test");',
      principal: { sub: "user1", info: null },
      action: { name: "read", service: "storage-service" },
      resource: {
        id: "https://example.com/folder/test",
        type: "folder",
        data: null,
      },
    },
    {
      id: 2,
      policy:
        'forbid(principal == Principal::"user2", action == Action::"storage-service:write", resource == folder::"https://example.com/folder/test");',
      principal: { sub: "user2", info: null },
      action: { name: "write", service: "storage-service" },
      resource: {
        id: "https://example.com/folder/test",
        type: "folder",
        data: null,
      },
    },
  ],
  page: 1,
  page_size: 2,
};

const mockPolicyPageWithConditions: PolicyPage = {
  items: [
    {
      id: 3,
      policy:
        'permit(principal == Principal::"user3", action == Action::"storage-service:read", resource == folder::"https://example.com/folder/test") when { true };',
      principal: { sub: "user3", info: null },
      action: { name: "read", service: "storage-service" },
      resource: {
        id: "https://example.com/folder/test",
        type: "folder",
        data: null,
      },
    },
    {
      id: 4,
      policy: "invalid-policy-format",
      principal: { sub: "user4", info: null },
      action: { name: "read", service: "storage-service" },
      resource: {
        id: "https://example.com/folder/test",
        type: "folder",
        data: null,
      },
    },
  ],
  page: 1,
  page_size: 2,
};

const server = setupServer();

beforeEach(() => {
  server.listen();
  vi.clearAllMocks();
});

afterEach(() => {
  server.resetHandlers();
});

describe("getPermissions", () => {
  it("should fetch permissions successfully", async () => {
    server.use(
      http.get("https://api.example.com/v1beta/policies/", ({ request }) => {
        const url = new URL(request.url);
        expect(url.searchParams.get("action")).toBeNull();
        expect(url.searchParams.get("resource")).toBe(
          'folder::"https://example.com/folder/test"',
        );
        expect(url.searchParams.get("limit")).toBe("50");
        expect(url.searchParams.get("page")).toBe("1");
        expect(request.headers.get("Authorization")).toBe(
          "Bearer test-access-token",
        );

        return HttpResponse.json(mockPolicyPage);
      }),
    );

    const params: GetPermissionsParams = {
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
    };

    const result = await getPermissions(params);

    expect(result).toEqual({
      entries: {
        user1: {
          read: { effect: "allow", policies: [mockPolicyPage.items[0]] },
          write: { effect: "unset" },
          edit: { effect: "unset" },
        },
        user2: {
          read: { effect: "unset" },
          write: { effect: "deny", policies: [mockPolicyPage.items[1]] },
          edit: { effect: "unset" },
        },
      },
      page: 1,
      unsupported: [],
    });
  });

  it("should parse a permissions:edit policy into the edit action", async () => {
    const editPolicy = {
      id: 20,
      policy:
        'permit(principal == Principal::"user1", action == Action::"permissions:edit", resource == folder::"https://example.com/folder/test");',
      principal: { sub: "user1", info: null },
      action: { name: "edit", service: "permissions" },
      resource: {
        id: "https://example.com/folder/test",
        type: "folder",
        data: null,
      },
    };

    server.use(
      http.get("https://api.example.com/v1beta/policies/", () => {
        return HttpResponse.json({
          items: [editPolicy],
          page: 1,
          page_size: 1,
        });
      }),
    );

    const result = await getPermissions({
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
    });

    expect(result.unsupported).toHaveLength(0);
    expect(result.entries.user1).toEqual({
      read: { effect: "unset" },
      write: { effect: "unset" },
      edit: { effect: "allow", policies: [editPolicy] },
    });
  });

  it("should discard policies for unrelated services", async () => {
    server.use(
      http.get("https://api.example.com/v1beta/policies/", () => {
        return HttpResponse.json({
          items: [
            {
              id: 21,
              policy:
                'permit(principal == Principal::"user1", action == Action::"permissions:view", resource == folder::"https://example.com/folder/test");',
              principal: { sub: "user1", info: null },
              action: { name: "view", service: "permissions" },
              resource: {
                id: "https://example.com/folder/test",
                type: "folder",
                data: null,
              },
            },
          ],
          page: 1,
          page_size: 1,
        });
      }),
    );

    const result = await getPermissions({
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
    });

    expect(result.unsupported).toHaveLength(0);
    expect(result.entries).toEqual({});
  });

  it("should support multiline policies", async () => {
    const policyText = `
      permit(
        principal == Principal::"user1", 
        action == Action::"storage-service:read", 
        resource == folder::"https://example.com/folder/test"
      );
    `;

    const policy = {
      id: 1,
      policy: policyText,
      principal: { sub: "user1", info: null },
      action: { name: "read", service: "storage-service" },
      resource: {
        id: "https://example.com/folder/test",
        type: "folder",
        data: null,
      },
    };

    server.use(
      http.get("https://api.example.com/v1beta/policies/", () => {
        return HttpResponse.json({
          items: [policy],
          page: 1,
          page_size: 1,
        });
      }),
    );

    const params: GetPermissionsParams = {
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
    };

    const result = await getPermissions(params);

    expect(result).toEqual({
      entries: {
        user1: {
          read: { effect: "allow", policies: [policy] },
          write: { effect: "unset" },
          edit: { effect: "unset" },
        },
      },
      page: 1,
      unsupported: [],
    });
  });

  it("should handle pagination correctly", async () => {
    const firstPage = { ...mockPolicyPage, page_size: 50 };
    const secondPage = { ...mockPolicyPage, page: 2, page_size: 2 };

    server.use(
      http.get("https://api.example.com/v1beta/policies/", ({ request }) => {
        const url = new URL(request.url);
        const page = url.searchParams.get("page");

        if (page === "1") {
          return HttpResponse.json(firstPage);
        } else if (page === "2") {
          return HttpResponse.json(secondPage);
        }

        return HttpResponse.json({ items: [], page: 3, page_size: 0 });
      }),
    );

    const params: GetPermissionsParams = {
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
    };

    const result = await getPermissions(params);

    expect(result.page).toBe(2);
  });

  it("should handle policies with conditions as unsupported", async () => {
    server.use(
      http.get("https://api.example.com/v1beta/policies/", () => {
        return HttpResponse.json(mockPolicyPageWithConditions);
      }),
    );

    const params: GetPermissionsParams = {
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
    };

    const result = await getPermissions(params);

    expect(result.unsupported).toHaveLength(2);
    expect(result.unsupported[0].id).toBe(3);
    expect(result.unsupported[1].id).toBe(4);
  });

  it("should handle 401 authentication error", async () => {
    server.use(
      http.get("https://api.example.com/v1beta/policies/", () => {
        return HttpResponse.text("Unauthorized", { status: 401 });
      }),
    );

    const params: GetPermissionsParams = {
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
    };

    await expect(getPermissions(params)).rejects.toThrow(
      "Failed to get permissions",
    );
    expect(mockAuth.onExpire).toHaveBeenCalled();
  });

  it("should handle other HTTP errors", async () => {
    server.use(
      http.get("https://api.example.com/v1beta/policies/", () => {
        return HttpResponse.text("Internal Server Error", { status: 500 });
      }),
    );

    const params: GetPermissionsParams = {
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
    };

    await expect(getPermissions(params)).rejects.toThrow(
      "Failed to get permissions",
    );
    expect(mockAuth.onExpire).not.toHaveBeenCalled();
  });

  it("should handle missing authorization token", async () => {
    server.use(
      http.get("https://api.example.com/v1beta/policies/", ({ request }) => {
        expect(request.headers.get("Authorization")).toBeNull();
        return HttpResponse.json(mockPolicyPage);
      }),
    );

    const params: GetPermissionsParams = {
      auth: { ...mockAuth, accessToken: "" },
      config: mockConfig,
      resource: mockResource,
    };

    const result = await getPermissions(params);
    expect(result).toBeDefined();
  });

  it("should handle policies with unknown actions", async () => {
    const pageWithUnknownAction: PolicyPage = {
      items: [
        {
          id: 5,
          policy:
            'permit(principal == Principal::"user5", action == Action::"storage-service:unknown", resource == folder::"https://example.com/folder/test");',
          principal: { sub: "user5", info: null },
          action: { name: "unknown", service: "storage-service" },
          resource: {
            id: "https://example.com/folder/test",
            type: "folder",
            data: null,
          },
        },
      ],
      page: 1,
      page_size: 1,
    };

    server.use(
      http.get("https://api.example.com/v1beta/policies/", () => {
        return HttpResponse.json(pageWithUnknownAction);
      }),
    );

    const params: GetPermissionsParams = {
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
    };

    const result = await getPermissions(params);

    expect(result.unsupported).toHaveLength(1);
    expect(result.unsupported[0].id).toBe(5);
  });

  it("should handle policies with missing principal/action/resource", async () => {
    const pageWithMissingFields: PolicyPage = {
      items: [
        {
          id: 6,
          policy: "permit();",
          principal: null,
          action: null,
          resource: null,
        },
      ],
      page: 1,
      page_size: 1,
    };

    server.use(
      http.get("https://api.example.com/v1beta/policies/", () => {
        return HttpResponse.json(pageWithMissingFields);
      }),
    );

    const params: GetPermissionsParams = {
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
    };

    const result = await getPermissions(params);

    expect(result.unsupported).toHaveLength(1);
    expect(result.unsupported[0].id).toBe(6);
  });

  it("shows containment-scoped policies as read-only entries, not unsupported", async () => {
    const containmentPolicy: Policy = {
      id: 8,
      policy:
        'permit(principal == Principal::"user8", action == Action::"storage-service:read", resource in folder::"https://example.com/folder/test");',
      principal: { sub: "user8", info: null },
      action: { name: "read", service: "storage-service" },
      resource: null,
    };

    server.use(
      http.get("https://api.example.com/v1beta/policies/", () => {
        return HttpResponse.json({
          items: [containmentPolicy],
          page: 1,
          page_size: 1,
        });
      }),
    );

    const result = await getPermissions({
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
    });

    expect(result.unsupported).toHaveLength(0);
    expect(result.entries.user8).toEqual({
      read: { effect: "allow", policies: [containmentPolicy], readonly: true },
      write: { effect: "unset" },
      edit: { effect: "unset" },
    });
  });

  it("keeps exact-scoped policies editable while marking containment ones read-only", async () => {
    const exactPolicy: Policy = {
      id: 9,
      policy:
        'permit(principal == Principal::"user9", action == Action::"storage-service:read", resource == folder::"https://example.com/folder/test");',
      principal: { sub: "user9", info: null },
      action: { name: "read", service: "storage-service" },
      resource: {
        id: "https://example.com/folder/test",
        type: "folder",
        data: null,
      },
    };
    const containmentPolicy: Policy = {
      id: 10,
      policy:
        'forbid(principal == Principal::"user9", action == Action::"storage-service:write", resource in folder::"https://example.com/folder/test");',
      principal: { sub: "user9", info: null },
      action: { name: "write", service: "storage-service" },
      resource: null,
    };

    server.use(
      http.get("https://api.example.com/v1beta/policies/", () => {
        return HttpResponse.json({
          items: [exactPolicy, containmentPolicy],
          page: 1,
          page_size: 2,
        });
      }),
    );

    const result = await getPermissions({
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
    });

    expect(result.entries.user9.read).toEqual({
      effect: "allow",
      policies: [exactPolicy],
    });
    expect(result.entries.user9.write).toEqual({
      effect: "deny",
      policies: [containmentPolicy],
      readonly: true,
    });
  });

  it("keeps a permissions:edit policy assigned on the selection editable", async () => {
    const editPolicy: Policy = {
      id: 11,
      policy:
        'permit(principal == Principal::"user11", action == Action::"permissions:edit", resource in folder::"https://example.com/folder/test");',
      principal: { sub: "user11", info: null },
      action: { name: "edit", service: "permissions" },
      resource: null,
      resource_prefix: {
        id: "https://example.com/folder/test",
        type: "folder",
        data: null,
      },
    };

    server.use(
      http.get("https://api.example.com/v1beta/policies/", () => {
        return HttpResponse.json({
          items: [editPolicy],
          page: 1,
          page_size: 1,
        });
      }),
    );

    const result = await getPermissions({
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
    });

    expect(result.entries.user11.edit).toEqual({
      effect: "allow",
      policies: [editPolicy],
    });
  });

  it("marks a SYSTEM-created permissions:edit policy as read-only", async () => {
    const systemEditPolicy = {
      id: 12,
      created_by: "SYSTEM",
      policy:
        'permit(principal == Principal::"user12", action == Action::"permissions:edit", resource in folder::"https://example.com/folder/test");',
      principal: { sub: "user12", info: null },
      action: { name: "edit", service: "permissions" },
      resource: null,
      resource_prefix: {
        id: "https://example.com/folder/test",
        type: "folder",
        data: null,
      },
    };

    server.use(
      http.get("https://api.example.com/v1beta/policies/", () => {
        return HttpResponse.json({
          items: [systemEditPolicy],
          page: 1,
          page_size: 1,
        });
      }),
    );

    const result = await getPermissions({
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
    });

    expect(result.entries.user12.edit?.readonly).toBe(true);
    expect(result.entries.user12.edit?.effect).toBe("allow");
    expect(result.entries.user12.edit?.policies).toHaveLength(1);
  });

  it("marks a SYSTEM-created exact-scoped policy as read-only", async () => {
    const systemPolicy = {
      id: 13,
      created_by: "SYSTEM",
      policy:
        'permit(principal == Principal::"user13", action == Action::"permissions:edit", resource == folder::"https://example.com/folder/test");',
      principal: { sub: "user13", info: null },
      action: { name: "edit", service: "permissions" },
      resource: {
        id: "https://example.com/folder/test",
        type: "folder",
        data: null,
      },
    };

    server.use(
      http.get("https://api.example.com/v1beta/policies/", () => {
        return HttpResponse.json({
          items: [systemPolicy],
          page: 1,
          page_size: 1,
        });
      }),
    );

    const result = await getPermissions({
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
    });

    expect(result.entries.user13.edit?.readonly).toBe(true);
  });

  it("should prioritize permit over forbid policies", async () => {
    const pageWithConflictingPolicies: PolicyPage = {
      items: [
        {
          id: 7,
          policy:
            'forbid(principal == Principal::"user7", action == Action::"storage-service:read", resource == folder::"https://example.com/folder/test");',
          principal: { sub: "user7", info: null },
          action: { name: "read", service: "storage-service" },
          resource: {
            id: "https://example.com/folder/test",
            type: "folder",
            data: null,
          },
        },
        {
          id: 8,
          policy:
            'permit(principal == Principal::"user7", action == Action::"storage-service:read", resource == folder::"https://example.com/folder/test");',
          principal: { sub: "user7", info: null },
          action: { name: "read", service: "storage-service" },
          resource: {
            id: "https://example.com/folder/test",
            type: "folder",
            data: null,
          },
        },
      ],
      page: 1,
      page_size: 2,
    };

    server.use(
      http.get("https://api.example.com/v1beta/policies/", () => {
        return HttpResponse.json(pageWithConflictingPolicies);
      }),
    );

    const params: GetPermissionsParams = {
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
    };

    const result = await getPermissions(params);

    expect(result.entries.user7!.read!.effect).toBe("allow");
    expect(
      result.entries.user7!.read!.policies!.map((p) => p.id).sort(),
    ).toEqual([7, 8]);
  });

  it("uses the order field to resolve conflicting policies", async () => {
    const pageWithOrderedPolicies: PolicyPage = {
      items: [
        {
          id: 7,
          order: 1,
          policy:
            'forbid(principal == Principal::"user7", action == Action::"storage-service:read", resource == folder::"https://example.com/folder/test");',
          principal: { sub: "user7", info: null },
          action: { name: "read", service: "storage-service" },
          resource: {
            id: "https://example.com/folder/test",
            type: "folder",
            data: null,
          },
        },
        {
          id: 8,
          order: 2,
          policy:
            'permit(principal == Principal::"user7", action == Action::"storage-service:read", resource == folder::"https://example.com/folder/test");',
          principal: { sub: "user7", info: null },
          action: { name: "read", service: "storage-service" },
          resource: {
            id: "https://example.com/folder/test",
            type: "folder",
            data: null,
          },
        },
      ],
      page: 1,
      page_size: 2,
    };

    server.use(
      http.get("https://api.example.com/v1beta/policies/", () => {
        return HttpResponse.json(pageWithOrderedPolicies);
      }),
    );

    const result = await getPermissions({
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
    });

    expect(result.entries.user7!.read!.effect).toBe("deny");
    expect(
      result.entries.user7!.read!.policies!.map((p) => p.id).sort(),
    ).toEqual([7, 8]);
  });

  it("prefers forbid over permit for permissions:edit at the same order", async () => {
    const prefix = {
      id: "https://example.com/folder/test",
      type: "folder" as const,
      data: null,
    };
    const page: PolicyPage = {
      items: [
        {
          id: 20,
          policy:
            'permit(principal == Principal::"user20", action == Action::"permissions:edit", resource in folder::"https://example.com/folder/test");',
          principal: { sub: "user20", info: null },
          action: { name: "edit", service: "permissions" },
          resource: null,
          resource_prefix: prefix,
        },
        {
          id: 21,
          policy:
            'forbid(principal == Principal::"user20", action == Action::"permissions:edit", resource in folder::"https://example.com/folder/test");',
          principal: { sub: "user20", info: null },
          action: { name: "edit", service: "permissions" },
          resource: null,
          resource_prefix: prefix,
        },
      ],
      page: 1,
      page_size: 2,
    };

    server.use(
      http.get("https://api.example.com/v1beta/policies/", () => {
        return HttpResponse.json(page);
      }),
    );

    const result = await getPermissions({
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
    });

    expect(result.entries.user20!.edit!.effect).toBe("deny");
    expect(
      result.entries.user20!.edit!.policies!.map((p) => p.id).sort(),
    ).toEqual([20, 21]);
  });

  it("should parse all-users policies without principal", async () => {
    const allUsersPolicy = {
      id: 10,
      policy:
        'permit(principal, action == Action::"storage-service:read", resource == folder::"https://example.com/folder/test");',
      principal: null,
      action: { name: "read", service: "storage-service" },
      resource: {
        id: "https://example.com/folder/test",
        type: "folder",
        data: null,
      },
    };

    server.use(
      http.get("https://api.example.com/v1beta/policies/", () => {
        return HttpResponse.json({
          items: [allUsersPolicy],
          page: 1,
          page_size: 1,
        });
      }),
    );

    const result = await getPermissions({
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
    });

    expect(result.entries[ALL_USERS]).toBeDefined();
    expect(result.entries[ALL_USERS]!.read!.effect).toBe("allow");
    expect(result.entries[ALL_USERS]!.read!.policies).toHaveLength(1);
    expect(Object.keys(result.entries)).toEqual([ALL_USERS]);
  });

  it("should treat all-users policies with conditions as unsupported", async () => {
    server.use(
      http.get("https://api.example.com/v1beta/policies/", () => {
        return HttpResponse.json({
          items: [
            {
              id: 11,
              policy:
                'permit(principal, action == Action::"storage-service:read", resource == folder::"https://example.com/folder/test") when { true };',
              principal: null,
              action: { name: "read", service: "storage-service" },
              resource: {
                id: "https://example.com/folder/test",
                type: "folder",
                data: null,
              },
            },
          ],
          page: 1,
          page_size: 1,
        });
      }),
    );

    const result = await getPermissions({
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
    });

    expect(result.entries[ALL_USERS]).toBeUndefined();
    expect(result.unsupported).toHaveLength(1);
  });

  it("should parse group-scoped policies under a group key", async () => {
    const groupPolicy = {
      id: 20,
      policy:
        'permit(principal in Group::"group-123", action == Action::"storage-service:read", resource == folder::"https://example.com/folder/test");',
      principal: null,
      group: { id: "group-123" },
      action: { name: "read", service: "storage-service" },
      resource: {
        id: "https://example.com/folder/test",
        type: "folder",
        data: null,
      },
    };

    server.use(
      http.get("https://api.example.com/v1beta/policies/", () => {
        return HttpResponse.json({
          items: [groupPolicy],
          page: 1,
          page_size: 1,
        });
      }),
    );

    const result = await getPermissions({
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
    });

    const key = groupKey("group-123");
    expect(result.entries[key]).toBeDefined();
    expect(result.entries[key]!.read!.effect).toBe("allow");
    expect(result.entries[key]!.read!.policies).toHaveLength(1);
    expect(Object.keys(result.entries)).toEqual([key]);
  });

  it("should parse both all-users and principal policies together", async () => {
    server.use(
      http.get("https://api.example.com/v1beta/policies/", () => {
        return HttpResponse.json({
          items: [
            {
              id: 12,
              policy:
                'permit(principal, action == Action::"storage-service:read", resource == folder::"https://example.com/folder/test");',
              principal: null,
              action: { name: "read", service: "storage-service" },
              resource: {
                id: "https://example.com/folder/test",
                type: "folder",
                data: null,
              },
            },
            {
              id: 13,
              policy:
                'permit(principal == Principal::"user1", action == Action::"storage-service:write", resource == folder::"https://example.com/folder/test");',
              principal: { sub: "user1", info: null },
              action: { name: "write", service: "storage-service" },
              resource: {
                id: "https://example.com/folder/test",
                type: "folder",
                data: null,
              },
            },
          ],
          page: 1,
          page_size: 2,
        });
      }),
    );

    const result = await getPermissions({
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
    });

    expect(result.entries[ALL_USERS]!.read!.effect).toBe("allow");
    expect(result.entries.user1!.write!.effect).toBe("allow");
  });
});

describe("getGlobalPolicies", () => {
  it("queries without an action and keeps only storage-relevant policies", async () => {
    server.use(
      http.get("https://api.example.com/v1beta/policies/", ({ request }) => {
        const url = new URL(request.url);
        expect(url.searchParams.get("resource")).toBe("NULL");
        expect(url.searchParams.get("action")).toBeNull();
        return HttpResponse.json({
          items: [
            {
              id: 1,
              policy: "permit(principal, action, resource);",
              principal: { sub: "user1", info: null },
              action: { name: "read", service: "storage-service" },
              resource: null,
            },
            {
              id: 2,
              policy:
                "permit(principal, action, resource) when { context.mfa == true };",
              principal: { sub: "user2", info: null },
              action: { name: "read", service: "storage-service" },
              resource: null,
            },
            {
              id: 3,
              policy: "invalid-policy",
              principal: null,
              action: { name: "manage", service: "storage-service" },
              resource: null,
            },
            {
              id: 4,
              policy: 'permit(principal in Group::"test", action, resource);',
              principal: null,
              group: { id: "test" },
              action: null,
              resource: null,
            },
            {
              id: 5,
              policy: "permit(principal, action, resource);",
              principal: { sub: "user5", info: null },
              action: { name: "read", service: "other-service" },
              resource: null,
            },
          ],
          page: 1,
          page_size: 5,
        });
      }),
    );

    const params: GetGlobalPoliciesParams = {
      auth: mockAuth,
      config: mockConfig,
    };

    const result = await getGlobalPolicies(params);

    expect(result.policies.map((p) => p.id)).toEqual([1, 2, 3, 4]);
    expect(result.unresolvable.map((p) => p.id)).toEqual([2, 3]);
  });

  it("follows pagination", async () => {
    server.use(
      http.get("https://api.example.com/v1beta/policies/", ({ request }) => {
        const page = new URL(request.url).searchParams.get("page");
        if (page === "1") {
          return HttpResponse.json({
            items: [
              {
                id: 1,
                policy: "invalid",
                principal: null,
                action: { name: "read", service: "storage-service" },
                resource: null,
              },
            ],
            page: 1,
            page_size: 50,
          });
        }
        return HttpResponse.json({ items: [], page: 2, page_size: 0 });
      }),
    );

    const result = await getGlobalPolicies({
      auth: mockAuth,
      config: mockConfig,
    });

    expect(result.policies).toHaveLength(1);
    expect(result.unresolvable).toHaveLength(1);
  });

  it("handles 401 authentication error", async () => {
    server.use(
      http.get("https://api.example.com/v1beta/policies/", () => {
        return HttpResponse.text("Unauthorized", { status: 401 });
      }),
    );

    await expect(
      getGlobalPolicies({ auth: mockAuth, config: mockConfig }),
    ).rejects.toThrow("Failed to get policies for NULL");
    expect(mockAuth.onExpire).toHaveBeenCalled();
  });
});

describe("submitPermissions", () => {
  it("should add new permit policy for new principal", async () => {
    let batchPayload: PolicyPayload[] | null = null;

    server.use(
      http.put(
        "https://api.example.com/v1beta/policies/batch/",
        async ({ request }) => {
          batchPayload = (await request.json()) as PolicyPayload[];
          expect(request.headers.get("Authorization")).toBe(
            "Bearer test-access-token",
          );
          expect(request.headers.get("Content-Type")).toBe("application/json");
          return HttpResponse.text("", { status: 200 });
        },
      ),
    );

    const params: SubmitPermissionsParams = {
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
      submitting: {
        user1: {
          read: { effect: "allow" },
        },
      },
      initial: {},
    };

    await submitPermissions(params);

    expect(batchPayload).toHaveLength(1);
    expect(batchPayload![0].policy).toBe(
      'permit(principal == Principal::"user1", action == Action::"storage-service:read", resource == folder::"https://example.com/folder/test");',
    );
    expect(batchPayload![0].principal).toBe("user1");
    expect(batchPayload![0].action.name).toBe("read");
    expect(batchPayload![0].action.service).toBe("storage-service");
    expect(batchPayload![0].resource.id).toBe(
      "https://example.com/folder/test",
    );
    expect(batchPayload![0].resource.type).toBe("folder");
  });

  it("should add a permit permissions:edit policy for a new principal", async () => {
    let batchPayload: PolicyPayload[] | null = null;

    server.use(
      http.put(
        "https://api.example.com/v1beta/policies/batch/",
        async ({ request }) => {
          batchPayload = (await request.json()) as PolicyPayload[];
          return HttpResponse.text("", { status: 200 });
        },
      ),
    );

    await submitPermissions({
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
      submitting: {
        user1: {
          edit: { effect: "allow" },
        },
      },
      initial: {},
    });

    expect(batchPayload).toHaveLength(1);
    expect(batchPayload![0].policy).toBe(
      'permit(principal == Principal::"user1", action == Action::"permissions:edit", resource in folder::"https://example.com/folder/test");',
    );
    expect(batchPayload![0].principal).toBe("user1");
    expect(batchPayload![0].action.name).toBe("edit");
    expect(batchPayload![0].action.service).toBe("permissions");
  });

  it("should add a forbid permissions:edit policy for a new principal", async () => {
    let batchPayload: PolicyPayload[] | null = null;

    server.use(
      http.put(
        "https://api.example.com/v1beta/policies/batch/",
        async ({ request }) => {
          batchPayload = (await request.json()) as PolicyPayload[];
          return HttpResponse.text("", { status: 200 });
        },
      ),
    );

    await submitPermissions({
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
      submitting: {
        user1: {
          edit: { effect: "deny" },
        },
      },
      initial: {},
    });

    expect(batchPayload).toHaveLength(1);
    expect(batchPayload![0].policy).toBe(
      'forbid(principal == Principal::"user1", action == Action::"permissions:edit", resource in folder::"https://example.com/folder/test");',
    );
    expect(batchPayload![0].action.name).toBe("edit");
    expect(batchPayload![0].action.service).toBe("permissions");
  });

  it("should add a group-scoped permit permissions:edit policy", async () => {
    let batchPayload: PolicyPayload[] | null = null;

    server.use(
      http.put(
        "https://api.example.com/v1beta/policies/batch/",
        async ({ request }) => {
          batchPayload = (await request.json()) as PolicyPayload[];
          return HttpResponse.text("", { status: 200 });
        },
      ),
    );

    await submitPermissions({
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
      submitting: {
        [groupKey("group-123")]: {
          edit: { effect: "allow" },
        },
      },
      initial: {},
    });

    expect(batchPayload).toHaveLength(1);
    expect(batchPayload![0].policy).toBe(
      'permit(principal in Group::"group-123", action == Action::"permissions:edit", resource in folder::"https://example.com/folder/test");',
    );
    expect(batchPayload![0].group).toBe("group-123");
  });

  it("should add an all-users permit permissions:edit policy", async () => {
    let batchPayload: PolicyPayload[] | null = null;

    server.use(
      http.put(
        "https://api.example.com/v1beta/policies/batch/",
        async ({ request }) => {
          batchPayload = (await request.json()) as PolicyPayload[];
          return HttpResponse.text("", { status: 200 });
        },
      ),
    );

    await submitPermissions({
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
      submitting: {
        [ALL_USERS]: {
          edit: { effect: "allow" },
        },
      },
      initial: {},
    });

    expect(batchPayload).toHaveLength(1);
    expect(batchPayload![0].policy).toBe(
      'permit(principal, action == Action::"permissions:edit", resource in folder::"https://example.com/folder/test");',
    );
    expect(batchPayload![0]).not.toHaveProperty("principal");
    expect(batchPayload![0]).not.toHaveProperty("group");
  });

  it("should scope a permissions:edit policy to an object with an exact match", async () => {
    let batchPayload: PolicyPayload[] | null = null;

    server.use(
      http.put(
        "https://api.example.com/v1beta/policies/batch/",
        async ({ request }) => {
          batchPayload = (await request.json()) as PolicyPayload[];
          return HttpResponse.text("", { status: 200 });
        },
      ),
    );

    await submitPermissions({
      auth: mockAuth,
      config: mockConfig,
      resource: {
        address: "https://example.com/folder/file.txt",
        type: "object",
      },
      submitting: {
        user1: {
          edit: { effect: "allow" },
        },
      },
      initial: {},
    });

    expect(batchPayload).toHaveLength(1);
    expect(batchPayload![0].policy).toBe(
      'permit(principal == Principal::"user1", action == Action::"permissions:edit", resource == object::"https://example.com/folder/file.txt");',
    );
  });

  it("should add a group-scoped permit policy with the group head and field", async () => {
    let batchPayload: PolicyPayload[] | null = null;

    server.use(
      http.put(
        "https://api.example.com/v1beta/policies/batch/",
        async ({ request }) => {
          batchPayload = (await request.json()) as PolicyPayload[];
          return HttpResponse.text("", { status: 200 });
        },
      ),
    );

    await submitPermissions({
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
      submitting: {
        [groupKey("group-123")]: {
          read: { effect: "allow" },
        },
      },
      initial: {},
    });

    expect(batchPayload).toHaveLength(1);
    expect(batchPayload![0].policy).toBe(
      'permit(principal in Group::"group-123", action == Action::"storage-service:read", resource == folder::"https://example.com/folder/test");',
    );
    expect(batchPayload![0].group).toBe("group-123");
    expect(batchPayload![0]).not.toHaveProperty("principal");
  });

  it("should add new forbid policy for new principal", async () => {
    let batchPayload: PolicyPayload[] | null = null;

    server.use(
      http.put(
        "https://api.example.com/v1beta/policies/batch/",
        async ({ request }) => {
          batchPayload = (await request.json()) as PolicyPayload[];
          return HttpResponse.text("", { status: 200 });
        },
      ),
    );

    const params: SubmitPermissionsParams = {
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
      submitting: {
        user1: {
          write: { effect: "deny" },
        },
      },
      initial: {},
    };

    await submitPermissions(params);

    expect(batchPayload).toHaveLength(1);
    expect(batchPayload![0].policy).toBe(
      'forbid(principal == Principal::"user1", action == Action::"storage-service:write", resource == folder::"https://example.com/folder/test");',
    );
    expect(batchPayload![0].principal).toBe("user1");
    expect(batchPayload![0].action.name).toBe("write");
  });

  it("should add both permit and forbid policies for new principal", async () => {
    let batchPayload: PolicyPayload[] | null = null;

    server.use(
      http.put(
        "https://api.example.com/v1beta/policies/batch/",
        async ({ request }) => {
          batchPayload = (await request.json()) as PolicyPayload[];
          return HttpResponse.text("", { status: 200 });
        },
      ),
    );

    const params: SubmitPermissionsParams = {
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
      submitting: {
        user1: {
          read: { effect: "allow" },
          write: { effect: "deny" },
        },
      },
      initial: {},
    };

    await submitPermissions(params);

    expect(batchPayload).toHaveLength(2);

    const permitPolicy = batchPayload!.find((p) =>
      p.policy.startsWith("permit"),
    );
    const forbidPolicy = batchPayload!.find((p) =>
      p.policy.startsWith("forbid"),
    );

    expect(permitPolicy?.policy).toBe(
      'permit(principal == Principal::"user1", action == Action::"storage-service:read", resource == folder::"https://example.com/folder/test");',
    );
    expect(forbidPolicy?.policy).toBe(
      'forbid(principal == Principal::"user1", action == Action::"storage-service:write", resource == folder::"https://example.com/folder/test");',
    );
  });

  it("should change from allow to deny by deleting old permit and adding forbid", async () => {
    const deletedPolicyIds: number[] = [];
    let batchPayload: PolicyPayload[] | null = null;

    server.use(
      http.delete(
        "https://api.example.com/v1beta/policies/:id",
        ({ params }) => {
          deletedPolicyIds.push(Number(params.id));
          return HttpResponse.text("", { status: 200 });
        },
      ),
      http.put(
        "https://api.example.com/v1beta/policies/batch/",
        async ({ request }) => {
          batchPayload = (await request.json()) as PolicyPayload[];
          return HttpResponse.text("", { status: 200 });
        },
      ),
    );

    const params: SubmitPermissionsParams = {
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
      submitting: {
        user1: {
          read: { effect: "deny" },
        },
      },
      initial: {
        user1: {
          read: { effect: "allow", policies: [{ id: 1 } as Policy] },
        },
      },
    };

    await submitPermissions(params);

    expect(deletedPolicyIds).toContain(1);
    expect(batchPayload).toHaveLength(1);
    expect(batchPayload![0].policy).toBe(
      'forbid(principal == Principal::"user1", action == Action::"storage-service:read", resource == folder::"https://example.com/folder/test");',
    );
  });

  it("should change from deny to allow by deleting old forbid and adding permit", async () => {
    const deletedPolicyIds: number[] = [];
    let batchPayload: PolicyPayload[] | null = null;

    server.use(
      http.delete(
        "https://api.example.com/v1beta/policies/:id",
        ({ params }) => {
          deletedPolicyIds.push(Number(params.id));
          return HttpResponse.text("", { status: 200 });
        },
      ),
      http.put(
        "https://api.example.com/v1beta/policies/batch/",
        async ({ request }) => {
          batchPayload = (await request.json()) as PolicyPayload[];
          return HttpResponse.text("", { status: 200 });
        },
      ),
    );

    const params: SubmitPermissionsParams = {
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
      submitting: {
        user1: {
          write: { effect: "allow" },
        },
      },
      initial: {
        user1: {
          write: { effect: "deny", policies: [{ id: 2 } as Policy] },
        },
      },
    };

    await submitPermissions(params);

    expect(deletedPolicyIds).toContain(2);
    expect(batchPayload).toHaveLength(1);
    expect(batchPayload![0].policy).toBe(
      'permit(principal == Principal::"user1", action == Action::"storage-service:write", resource == folder::"https://example.com/folder/test");',
    );
  });

  it("should handle multiple permission changes for same principal", async () => {
    const deletedPolicyIds: number[] = [];
    let batchPayload: PolicyPayload[] | null = null;

    server.use(
      http.delete(
        "https://api.example.com/v1beta/policies/:id",
        ({ params }) => {
          deletedPolicyIds.push(Number(params.id));
          return HttpResponse.text("", { status: 200 });
        },
      ),
      http.put(
        "https://api.example.com/v1beta/policies/batch/",
        async ({ request }) => {
          batchPayload = (await request.json()) as PolicyPayload[];
          return HttpResponse.text("", { status: 200 });
        },
      ),
    );

    const params: SubmitPermissionsParams = {
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
      submitting: {
        user1: {
          read: { effect: "deny" },
          write: { effect: "allow" },
        },
      },
      initial: {
        user1: {
          read: { effect: "allow", policies: [{ id: 1 } as Policy] },
          write: { effect: "deny", policies: [{ id: 2 } as Policy] },
        },
      },
    };

    await submitPermissions(params);

    expect(deletedPolicyIds).toContain(1);
    expect(deletedPolicyIds).toContain(2);
    expect(batchPayload).toHaveLength(2);

    const forbidPolicy = batchPayload!.find((p) =>
      p.policy.startsWith("forbid"),
    );
    const permitPolicy = batchPayload!.find((p) =>
      p.policy.startsWith("permit"),
    );

    expect(forbidPolicy?.policy).toBe(
      'forbid(principal == Principal::"user1", action == Action::"storage-service:read", resource == folder::"https://example.com/folder/test");',
    );
    expect(permitPolicy?.policy).toBe(
      'permit(principal == Principal::"user1", action == Action::"storage-service:write", resource == folder::"https://example.com/folder/test");',
    );
  });

  it("should remove old permit policy when changing allow to unset", async () => {
    const deletedPolicyIds: number[] = [];
    let batchPayload: PolicyPayload[] | null = null;

    server.use(
      http.delete(
        "https://api.example.com/v1beta/policies/:id",
        ({ params }) => {
          deletedPolicyIds.push(Number(params.id));
          return HttpResponse.text("", { status: 200 });
        },
      ),
      http.put(
        "https://api.example.com/v1beta/policies/batch/",
        async ({ request }) => {
          batchPayload = (await request.json()) as PolicyPayload[];
          return HttpResponse.text("", { status: 200 });
        },
      ),
    );

    await submitPermissions({
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
      submitting: {
        user1: {
          read: { effect: "unset" },
        },
      },
      initial: {
        user1: {
          read: { effect: "allow", policies: [{ id: 1 } as Policy] },
        },
      },
    });

    expect(deletedPolicyIds).toContain(1);
    expect(batchPayload).toHaveLength(0);
  });

  it("should remove old forbid policy when changing deny to unset", async () => {
    const deletedPolicyIds: number[] = [];
    let batchPayload: PolicyPayload[] | null = null;

    server.use(
      http.delete(
        "https://api.example.com/v1beta/policies/:id",
        ({ params }) => {
          deletedPolicyIds.push(Number(params.id));
          return HttpResponse.text("", { status: 200 });
        },
      ),
      http.put(
        "https://api.example.com/v1beta/policies/batch/",
        async ({ request }) => {
          batchPayload = (await request.json()) as PolicyPayload[];
          return HttpResponse.text("", { status: 200 });
        },
      ),
    );

    await submitPermissions({
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
      submitting: {
        user1: {
          write: { effect: "unset" },
        },
      },
      initial: {
        user1: {
          write: { effect: "deny", policies: [{ id: 2 } as Policy] },
        },
      },
    });

    expect(deletedPolicyIds).toContain(2);
    expect(batchPayload).toHaveLength(0);
  });

  it("should not create any policy for a new unset permission", async () => {
    const deletedPolicyIds: number[] = [];
    let batchPayload: PolicyPayload[] | null = null;

    server.use(
      http.delete(
        "https://api.example.com/v1beta/policies/:id",
        ({ params }) => {
          deletedPolicyIds.push(Number(params.id));
          return HttpResponse.text("", { status: 200 });
        },
      ),
      http.put(
        "https://api.example.com/v1beta/policies/batch/",
        async ({ request }) => {
          batchPayload = (await request.json()) as PolicyPayload[];
          return HttpResponse.text("", { status: 200 });
        },
      ),
    );

    await submitPermissions({
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
      submitting: {
        user1: {
          read: { effect: "unset" },
          write: { effect: "unset" },
        },
      },
      initial: {},
    });

    expect(deletedPolicyIds).toHaveLength(0);
    expect(batchPayload).toHaveLength(0);
  });

  it("should change from deny to allow when no old policies exist", async () => {
    let batchPayload: PolicyPayload[] | null = null;

    server.use(
      http.put(
        "https://api.example.com/v1beta/policies/batch/",
        async ({ request }) => {
          batchPayload = (await request.json()) as PolicyPayload[];
          return HttpResponse.text("", { status: 200 });
        },
      ),
    );

    const params: SubmitPermissionsParams = {
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
      submitting: {
        user1: {
          read: { effect: "allow" },
        },
      },
      initial: {
        user1: {
          read: { effect: "deny" },
        },
      },
    };

    await submitPermissions(params);

    expect(batchPayload).toHaveLength(1);
    expect(batchPayload![0].policy).toBe(
      'permit(principal == Principal::"user1", action == Action::"storage-service:read", resource == folder::"https://example.com/folder/test");',
    );
  });

  it("should change from allow to deny when no old policies exist", async () => {
    let batchPayload: PolicyPayload[] | null = null;

    server.use(
      http.put(
        "https://api.example.com/v1beta/policies/batch/",
        async ({ request }) => {
          batchPayload = (await request.json()) as PolicyPayload[];
          return HttpResponse.text("", { status: 200 });
        },
      ),
    );

    const params: SubmitPermissionsParams = {
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
      submitting: {
        user1: {
          write: { effect: "deny" },
        },
      },
      initial: {
        user1: {
          write: { effect: "allow" },
        },
      },
    };

    await submitPermissions(params);

    expect(batchPayload).toHaveLength(1);
    expect(batchPayload![0].policy).toBe(
      'forbid(principal == Principal::"user1", action == Action::"storage-service:write", resource == folder::"https://example.com/folder/test");',
    );
  });

  it("should delete policies when removing a principal", async () => {
    const deletedPolicyIds: number[] = [];

    server.use(
      http.delete(
        "https://api.example.com/v1beta/policies/:id",
        ({ params }) => {
          deletedPolicyIds.push(Number(params.id));
          return HttpResponse.text("", { status: 200 });
        },
      ),
      http.put("https://api.example.com/v1beta/policies/batch/", () => {
        return HttpResponse.text("", { status: 200 });
      }),
    );

    const params: SubmitPermissionsParams = {
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
      submitting: {},
      initial: {
        user1: {
          read: { effect: "allow", policies: [{ id: 1 } as Policy] },
          write: { effect: "deny", policies: [{ id: 2 } as Policy] },
        },
      },
    };

    await submitPermissions(params);

    expect(deletedPolicyIds).toContain(1);
    expect(deletedPolicyIds).toContain(2);
  });

  it("should handle multiple principals with different changes", async () => {
    const deletedPolicyIds: number[] = [];
    let batchPayload: PolicyPayload[] | null = null;

    server.use(
      http.delete(
        "https://api.example.com/v1beta/policies/:id",
        ({ params }) => {
          deletedPolicyIds.push(Number(params.id));
          return HttpResponse.text("", { status: 200 });
        },
      ),
      http.put(
        "https://api.example.com/v1beta/policies/batch/",
        async ({ request }) => {
          batchPayload = (await request.json()) as PolicyPayload[];
          return HttpResponse.text("", { status: 200 });
        },
      ),
    );

    const params: SubmitPermissionsParams = {
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
      submitting: {
        user1: {
          read: { effect: "deny" },
        },
        user2: {
          write: { effect: "allow" },
        },
      },
      initial: {
        user1: {
          read: { effect: "allow", policies: [{ id: 1 } as Policy] },
        },
        user3: {
          read: { effect: "allow", policies: [{ id: 2 } as Policy] },
        },
      },
    };

    await submitPermissions(params);

    expect(deletedPolicyIds).toContain(1);
    expect(deletedPolicyIds).toContain(2);
    expect(batchPayload).toHaveLength(2);

    const user1Policy = batchPayload!.find((p) => p.principal === "user1");
    const user2Policy = batchPayload!.find((p) => p.principal === "user2");

    expect(user1Policy?.policy).toBe(
      'forbid(principal == Principal::"user1", action == Action::"storage-service:read", resource == folder::"https://example.com/folder/test");',
    );
    expect(user2Policy?.policy).toBe(
      'permit(principal == Principal::"user2", action == Action::"storage-service:write", resource == folder::"https://example.com/folder/test");',
    );
  });

  it("should handle delete policy 401 error", async () => {
    server.use(
      http.delete("https://api.example.com/v1beta/policies/:id", () => {
        return HttpResponse.text("Unauthorized", { status: 401 });
      }),
      http.put("https://api.example.com/v1beta/policies/batch/", () => {
        return HttpResponse.text("", { status: 200 });
      }),
    );

    const params: SubmitPermissionsParams = {
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
      submitting: {},
      initial: {
        user1: {
          read: { effect: "allow", policies: [{ id: 1 } as Policy] },
        },
      },
    };

    await expect(submitPermissions(params)).rejects.toThrow(
      "Failed to delete policy",
    );
    expect(mockAuth.onExpire).toHaveBeenCalled();
  });

  it("should handle batch submission 401 error", async () => {
    server.use(
      http.put("https://api.example.com/v1beta/policies/batch/", () => {
        return HttpResponse.text("Unauthorized", { status: 401 });
      }),
    );

    const params: SubmitPermissionsParams = {
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
      submitting: {
        user1: { read: { effect: "allow" } },
      },
      initial: {},
    };

    await expect(submitPermissions(params)).rejects.toThrow(
      "Failed to submit new policies",
    );
    expect(mockAuth.onExpire).toHaveBeenCalled();
  });

  it("should handle batch submission other errors", async () => {
    server.use(
      http.put("https://api.example.com/v1beta/policies/batch/", () => {
        return HttpResponse.text("Internal Server Error", { status: 500 });
      }),
    );

    const params: SubmitPermissionsParams = {
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
      submitting: {
        user1: { read: { effect: "allow" } },
      },
      initial: {},
    };

    await expect(submitPermissions(params)).rejects.toThrow(
      "Failed to submit new policies",
    );
    expect(mockAuth.onExpire).not.toHaveBeenCalled();
  });

  it("should not make changes when permissions are identical", async () => {
    let batchCalled = false;
    let deleteCalled = false;

    server.use(
      http.delete("https://api.example.com/v1beta/policies/:id", () => {
        deleteCalled = true;
        return HttpResponse.text("", { status: 200 });
      }),
      http.put(
        "https://api.example.com/v1beta/policies/batch/",
        async ({ request }) => {
          const payload = (await request.json()) as PolicyPayload[];
          batchCalled = true;
          expect(payload).toHaveLength(0);
          return HttpResponse.text("", { status: 200 });
        },
      ),
    );

    const params: SubmitPermissionsParams = {
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
      submitting: {
        user1: {
          read: { effect: "allow" },
          write: { effect: "deny" },
        },
      },
      initial: {
        user1: {
          read: { effect: "allow", policies: [{ id: 1 } as Policy] },
          write: { effect: "deny", policies: [{ id: 2 } as Policy] },
        },
      },
    };

    await submitPermissions(params);

    expect(deleteCalled).toBe(false);
    expect(batchCalled).toBe(true);
  });

  it("should submit all-users permit policy", async () => {
    let batchPayload: PolicyPayload[] | null = null;

    server.use(
      http.put(
        "https://api.example.com/v1beta/policies/batch/",
        async ({ request }) => {
          batchPayload = (await request.json()) as PolicyPayload[];
          return HttpResponse.text("", { status: 200 });
        },
      ),
    );

    await submitPermissions({
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
      submitting: { [ALL_USERS]: { read: { effect: "allow" } } },
      initial: {},
    });

    expect(batchPayload).toHaveLength(1);
    expect(batchPayload![0].policy).toBe(
      'permit(principal, action == Action::"storage-service:read", resource == folder::"https://example.com/folder/test");',
    );
    expect(batchPayload![0]).not.toHaveProperty("principal");
  });

  it("should submit all-users forbid policy", async () => {
    let batchPayload: PolicyPayload[] | null = null;

    server.use(
      http.put(
        "https://api.example.com/v1beta/policies/batch/",
        async ({ request }) => {
          batchPayload = (await request.json()) as PolicyPayload[];
          return HttpResponse.text("", { status: 200 });
        },
      ),
    );

    await submitPermissions({
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
      submitting: { [ALL_USERS]: { write: { effect: "deny" } } },
      initial: {},
    });

    expect(batchPayload).toHaveLength(1);
    expect(batchPayload![0].policy).toBe(
      'forbid(principal, action == Action::"storage-service:write", resource == folder::"https://example.com/folder/test");',
    );
    expect(batchPayload![0]).not.toHaveProperty("principal");
  });

  it("should delete old all-users policies when changing", async () => {
    const deletedPolicyIds: number[] = [];
    let batchPayload: PolicyPayload[] | null = null;

    server.use(
      http.delete(
        "https://api.example.com/v1beta/policies/:id",
        ({ params }) => {
          deletedPolicyIds.push(Number(params.id));
          return HttpResponse.text("", { status: 200 });
        },
      ),
      http.put(
        "https://api.example.com/v1beta/policies/batch/",
        async ({ request }) => {
          batchPayload = (await request.json()) as PolicyPayload[];
          return HttpResponse.text("", { status: 200 });
        },
      ),
    );

    await submitPermissions({
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
      submitting: { [ALL_USERS]: { read: { effect: "deny" } } },
      initial: {
        [ALL_USERS]: {
          read: { effect: "allow", policies: [{ id: 100 } as Policy] },
        },
      },
    });

    expect(deletedPolicyIds).toContain(100);
    expect(batchPayload!.some((p) => p.policy.includes("forbid"))).toBe(true);
  });

  it("should delete all-users policies when removing", async () => {
    const deletedPolicyIds: number[] = [];

    server.use(
      http.delete(
        "https://api.example.com/v1beta/policies/:id",
        ({ params }) => {
          deletedPolicyIds.push(Number(params.id));
          return HttpResponse.text("", { status: 200 });
        },
      ),
      http.put("https://api.example.com/v1beta/policies/batch/", () => {
        return HttpResponse.text("", { status: 200 });
      }),
    );

    await submitPermissions({
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
      submitting: {},
      initial: {
        [ALL_USERS]: {
          read: { effect: "allow", policies: [{ id: 101 } as Policy] },
          write: { effect: "deny", policies: [{ id: 102 } as Policy] },
        },
      },
    });

    expect(deletedPolicyIds).toContain(101);
    expect(deletedPolicyIds).toContain(102);
  });

  it("should create forbid policies for all-users when read and write are not set", async () => {
    let batchPayload: PolicyPayload[] | null = null;

    server.use(
      http.put(
        "https://api.example.com/v1beta/policies/batch/",
        async ({ request }) => {
          batchPayload = (await request.json()) as PolicyPayload[];
          return HttpResponse.text("", { status: 200 });
        },
      ),
    );

    await submitPermissions({
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
      submitting: {
        [ALL_USERS]: {
          read: { effect: "deny" },
          write: { effect: "deny" },
        },
      },
      initial: {},
    });

    expect(batchPayload).toHaveLength(2);
    const readPolicy = batchPayload!.find((p) =>
      p.policy.includes("storage-service:read"),
    );
    const writePolicy = batchPayload!.find((p) =>
      p.policy.includes("storage-service:write"),
    );
    expect(readPolicy).toBeDefined();
    expect(readPolicy!.policy).toMatch(/^forbid/);
    expect(readPolicy!).not.toHaveProperty("principal");
    expect(writePolicy).toBeDefined();
    expect(writePolicy!.policy).toMatch(/^forbid/);
    expect(writePolicy!).not.toHaveProperty("principal");
  });

  it("should create permit read and forbid write for all-users", async () => {
    let batchPayload: PolicyPayload[] | null = null;

    server.use(
      http.put(
        "https://api.example.com/v1beta/policies/batch/",
        async ({ request }) => {
          batchPayload = (await request.json()) as PolicyPayload[];
          return HttpResponse.text("", { status: 200 });
        },
      ),
    );

    await submitPermissions({
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
      submitting: {
        [ALL_USERS]: {
          read: { effect: "allow" },
          write: { effect: "deny" },
        },
      },
      initial: {},
    });

    expect(batchPayload).toHaveLength(2);
    const readPolicy = batchPayload!.find((p) =>
      p.policy.includes("storage-service:read"),
    );
    const writePolicy = batchPayload!.find((p) =>
      p.policy.includes("storage-service:write"),
    );
    expect(readPolicy).toBeDefined();
    expect(readPolicy!.policy).toMatch(/^permit/);
    expect(readPolicy!).not.toHaveProperty("principal");
    expect(writePolicy).toBeDefined();
    expect(writePolicy!.policy).toMatch(/^forbid/);
    expect(writePolicy!).not.toHaveProperty("principal");
  });

  it("should update all-users read from false to true while keeping write unchanged", async () => {
    const deletedPolicyIds: number[] = [];
    let batchPayload: PolicyPayload[] | null = null;

    server.use(
      http.delete(
        "https://api.example.com/v1beta/policies/:id",
        ({ params }) => {
          deletedPolicyIds.push(Number(params.id));
          return HttpResponse.text("", { status: 200 });
        },
      ),
      http.put(
        "https://api.example.com/v1beta/policies/batch/",
        async ({ request }) => {
          batchPayload = (await request.json()) as PolicyPayload[];
          return HttpResponse.text("", { status: 200 });
        },
      ),
    );

    await submitPermissions({
      auth: mockAuth,
      config: mockConfig,
      resource: mockResource,
      submitting: {
        [ALL_USERS]: {
          read: { effect: "allow" },
          write: { effect: "deny" },
        },
      },
      initial: {
        [ALL_USERS]: {
          read: { effect: "deny", policies: [{ id: 200 } as Policy] },
          write: { effect: "deny", policies: [{ id: 201 } as Policy] },
        },
      },
    });

    expect(deletedPolicyIds).toContain(200);
    expect(deletedPolicyIds).not.toContain(201);
    expect(batchPayload).toHaveLength(1);
    expect(batchPayload![0].policy).toMatch(/^permit/);
    expect(batchPayload![0].policy).toContain("storage-service:read");
    expect(batchPayload![0]).not.toHaveProperty("principal");
  });
});
