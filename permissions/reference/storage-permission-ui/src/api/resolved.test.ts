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

import {
  afterEach,
  beforeAll,
  afterAll,
  describe,
  expect,
  it,
  vi,
} from "vitest";
import { setupServer } from "msw/node";
import { http, HttpResponse } from "msw";
import {
  getResolvedPermissions,
  resolvePermissions,
  GLOBAL_SOURCE,
  type Grant,
  type Membership,
} from "./resolved.js";
import { ALL_USERS, groupKey } from "./policies.js";
import type { PermissionAuth } from "../types/PermissionAuth";
import type { PermissionConfig } from "../types/PermissionConfig";
import type { ResourceAddress } from "../types/ResourceAddress";

const HOST = "https://exampleaccount.blob.core.windows.net";
const CONTENT = `${HOST}/container/folder/`;
const MAIN = `${HOST}/container/`;
const ROOT = `${HOST}/`;

type LevelEntries = {
  [key: string]: {
    read?: "allow" | "deny";
    write?: "allow" | "deny";
    edit?: "allow" | "deny";
  };
};

function levelGrants(
  source: string,
  entries: LevelEntries,
  order = 100,
): Grant[] {
  const grants: Grant[] = [];
  for (const [key, actions] of Object.entries(entries)) {
    if (actions.read) {
      grants.push({ key, action: "read", effect: actions.read, order, source });
    }
    if (actions.write) {
      grants.push({
        key,
        action: "write",
        effect: actions.write,
        order,
        source,
      });
    }
    if (actions.edit) {
      grants.push({ key, action: "edit", effect: actions.edit, order, source });
    }
  }
  return grants;
}

describe("resolvePermissions", () => {
  it("resolves the three-level example bottom-to-top so a closer forbid wins", () => {
    const grants = [
      ...levelGrants(CONTENT, {
        bob: { read: "allow", write: "allow" },
        alice: { read: "deny", write: "deny" },
      }),
      ...levelGrants(MAIN, {
        [groupKey("artists")]: { read: "allow" },
        mary: { read: "allow", write: "allow" },
      }),
      ...levelGrants(ROOT, {
        [ALL_USERS]: { read: "allow", write: "deny" },
      }),
    ];
    const membership: Membership = new Map([
      ["bob", new Set(["artists"])],
      ["alice", new Set(["artists"])],
    ]);

    const result = resolvePermissions(grants, membership, [
      CONTENT,
      MAIN,
      ROOT,
    ]);

    expect(result.entries.bob).toEqual({
      read: { effect: "allow", source: CONTENT },
      write: { effect: "allow", source: CONTENT },
      edit: { effect: "unset" },
    });
    expect(result.entries.alice).toEqual({
      read: { effect: "deny", source: CONTENT },
      write: { effect: "deny", source: CONTENT },
      edit: { effect: "unset" },
    });
    expect(result.entries.mary).toEqual({
      read: { effect: "allow", source: MAIN },
      write: { effect: "allow", source: MAIN },
      edit: { effect: "unset" },
    });
    expect(result.entries[groupKey("artists")]).toEqual({
      read: { effect: "allow", source: MAIN },
      write: { effect: "deny", source: ROOT, viaAllUsers: true },
      edit: { effect: "unset" },
    });
    expect(result.entries[ALL_USERS]).toEqual({
      read: { effect: "allow", source: ROOT },
      write: { effect: "deny", source: ROOT },
      edit: { effect: "unset" },
    });
  });

  it("resolves an edit (ACL) policy set on the selection", () => {
    const grants = levelGrants(CONTENT, { bob: { edit: "allow" } });

    const result = resolvePermissions(grants, new Map(), [CONTENT, MAIN]);

    expect(result.entries.bob.edit).toEqual({
      effect: "allow",
      source: CONTENT,
    });
  });

  it("does not inherit an exact edit (ACL) policy from an ancestor folder", () => {
    const grants = [
      ...levelGrants(CONTENT, { bob: { read: "allow" } }),
      ...levelGrants(MAIN, { bob: { edit: "allow" } }),
    ];

    const result = resolvePermissions(grants, new Map(), [CONTENT, MAIN]);

    expect(result.entries.bob).toEqual({
      read: { effect: "allow", source: CONTENT },
      write: { effect: "unset" },
      edit: { effect: "unset" },
    });
  });

  it("applies an edit (ACL) policy that cascades from a parent via a resource prefix", () => {
    const grants: Grant[] = [
      {
        key: "bob",
        action: "edit",
        effect: "allow",
        order: 100,
        source: CONTENT,
        origin: MAIN,
      },
    ];

    const result = resolvePermissions(grants, new Map(), [CONTENT, MAIN]);

    expect(result.entries.bob.edit).toEqual({
      effect: "allow",
      source: MAIN,
      overridden: true,
    });
  });

  it("lets a forbid win over a permit for edit (ACL) at the same order", () => {
    const grants: Grant[] = [
      {
        key: "bob",
        action: "edit",
        effect: "allow",
        order: 100,
        source: CONTENT,
      },
      {
        key: "bob",
        action: "edit",
        effect: "deny",
        order: 100,
        source: CONTENT,
      },
    ];

    const result = resolvePermissions(grants, new Map(), [CONTENT]);

    expect(result.entries.bob.edit).toEqual({
      effect: "deny",
      source: CONTENT,
    });
  });

  it("still lets a permit win over a forbid for storage actions at the same order", () => {
    const grants: Grant[] = [
      {
        key: "bob",
        action: "read",
        effect: "deny",
        order: 100,
        source: CONTENT,
      },
      {
        key: "bob",
        action: "read",
        effect: "allow",
        order: 100,
        source: CONTENT,
      },
    ];

    const result = resolvePermissions(grants, new Map(), [CONTENT]);

    expect(result.entries.bob.read).toEqual({
      effect: "allow",
      source: CONTENT,
    });
  });

  it("prefers a closer forbid over a permit inherited from a parent", () => {
    const grants = [
      ...levelGrants(CONTENT, {
        [ALL_USERS]: { read: "deny", write: "deny" },
      }),
      ...levelGrants(MAIN, { [ALL_USERS]: { read: "allow" } }),
    ];

    const result = resolvePermissions(grants, new Map(), [CONTENT, MAIN]);

    expect(result.entries[ALL_USERS]).toEqual({
      read: { effect: "deny", source: CONTENT },
      write: { effect: "deny", source: CONTENT },
      edit: { effect: "unset" },
    });
  });

  it("inherits a missing action from the user's group", () => {
    const grants = levelGrants(CONTENT, {
      bob: { read: "allow" },
      [groupKey("artists")]: { read: "allow", write: "allow" },
    });
    const membership: Membership = new Map([["bob", new Set(["artists"])]]);

    const result = resolvePermissions(grants, membership, [CONTENT]);

    expect(result.entries.bob).toEqual({
      read: { effect: "allow", source: CONTENT },
      write: { effect: "allow", source: CONTENT, viaGroup: "artists" },
      edit: { effect: "unset" },
    });
    expect(result.entries[groupKey("artists")]).toEqual({
      read: { effect: "allow", source: CONTENT },
      write: { effect: "allow", source: CONTENT },
      edit: { effect: "unset" },
    });
  });

  it("inherits from the user's group defined on a different resource", () => {
    const grants = [
      ...levelGrants(CONTENT, { bob: { read: "allow" } }),
      ...levelGrants(MAIN, {
        [groupKey("artists")]: { read: "allow", write: "allow" },
      }),
    ];
    const membership: Membership = new Map([["bob", new Set(["artists"])]]);

    const result = resolvePermissions(grants, membership, [CONTENT, MAIN]);

    expect(result.entries.bob).toEqual({
      read: { effect: "allow", source: CONTENT },
      write: { effect: "allow", source: MAIN, viaGroup: "artists" },
      edit: { effect: "unset" },
    });
  });

  it("gives no principal-tier precedence, so an all-users permit wins over a group forbid at the same order", () => {
    const grants = levelGrants(CONTENT, {
      bob: { read: "allow" },
      [groupKey("artists")]: { write: "deny" },
      [ALL_USERS]: { write: "allow" },
    });
    const membership: Membership = new Map([["bob", new Set(["artists"])]]);

    const result = resolvePermissions(grants, membership, [CONTENT]);

    expect(result.entries.bob.write).toEqual({
      effect: "allow",
      source: CONTENT,
      viaAllUsers: true,
    });
  });

  it("lets permit win over forbid across groups at the same order", () => {
    const grants = levelGrants(CONTENT, {
      bob: { read: "allow" },
      [groupKey("a")]: { write: "deny" },
      [groupKey("b")]: { write: "allow" },
    });
    const membership: Membership = new Map([["bob", new Set(["a", "b"])]]);

    const result = resolvePermissions(grants, membership, [CONTENT]);

    expect(result.entries.bob.write).toEqual({
      effect: "allow",
      source: CONTENT,
      viaGroup: "b",
    });
  });

  it("falls back to all users when the user has no own or group policy", () => {
    const grants = [
      ...levelGrants(CONTENT, { bob: { read: "allow" } }),
      ...levelGrants(ROOT, { [ALL_USERS]: { read: "allow", write: "deny" } }),
    ];
    const membership: Membership = new Map([["bob", new Set()]]);

    const result = resolvePermissions(grants, membership, [CONTENT, ROOT]);

    expect(result.entries.bob).toEqual({
      read: { effect: "allow", source: CONTENT },
      write: { effect: "deny", source: ROOT, viaAllUsers: true },
      edit: { effect: "unset" },
    });
  });

  it("evaluates a prefix policy at the level but shows its parent folder as the source", () => {
    const grants: Grant[] = [
      {
        key: "user",
        action: "read",
        effect: "allow",
        order: 100,
        source: CONTENT,
        origin: MAIN,
      },
      {
        key: "user",
        action: "read",
        effect: "deny",
        order: 500,
        source: CONTENT,
        origin: CONTENT,
      },
    ];

    const result = resolvePermissions(grants, new Map(), [CONTENT]);

    expect(result.entries.user.read).toEqual({
      effect: "allow",
      source: MAIN,
      overridden: true,
    });
  });

  it("leaves a permission unset when nothing applies", () => {
    const grants = levelGrants(CONTENT, { bob: { read: "allow" } });

    const result = resolvePermissions(grants, new Map(), [CONTENT]);

    expect(result.entries.bob.write).toEqual({ effect: "unset" });
  });

  it("does not expand groups when membership is missing", () => {
    const grants = [
      ...levelGrants(CONTENT, { bob: { read: "allow" } }),
      ...levelGrants(MAIN, {
        [groupKey("artists")]: { read: "allow", write: "allow" },
      }),
    ];

    const result = resolvePermissions(grants, new Map(), [CONTENT, MAIN]);

    expect(result.entries.bob.write).toEqual({ effect: "unset" });
  });

  it("lets a global lower order win over a closer resource policy", () => {
    const grants = [
      ...levelGrants(CONTENT, { bob: { read: "deny" } }, 50),
      ...levelGrants(GLOBAL_SOURCE, { bob: { read: "allow" } }, 10),
    ];

    const result = resolvePermissions(grants, new Map(), [CONTENT]);

    expect(result.entries.bob.read).toEqual({
      effect: "allow",
      source: GLOBAL_SOURCE,
    });
  });

  it("lets a lower order win regardless of principal at the same level", () => {
    const grants = [
      ...levelGrants(CONTENT, { [ALL_USERS]: { read: "allow" } }, 10),
      ...levelGrants(CONTENT, { bob: { read: "deny" } }, 50),
    ];

    const result = resolvePermissions(grants, new Map(), [CONTENT]);

    expect(result.entries.bob.read).toEqual({
      effect: "allow",
      source: CONTENT,
      viaAllUsers: true,
    });
  });

  it("gives global and resource-scoped policies equal standing at the same level and order", () => {
    const globalPermitWins = [
      ...levelGrants(CONTENT, { bob: { read: "deny" } }),
      ...levelGrants(GLOBAL_SOURCE, { bob: { read: "allow" } }),
    ];
    expect(
      resolvePermissions(globalPermitWins, new Map(), [CONTENT]).entries.bob
        .read,
    ).toEqual({ effect: "allow", source: GLOBAL_SOURCE });

    const resourcePermitWins = [
      ...levelGrants(GLOBAL_SOURCE, { bob: { write: "deny" } }),
      ...levelGrants(CONTENT, { bob: { write: "allow" } }),
    ];
    expect(
      resolvePermissions(resourcePermitWins, new Map(), [CONTENT]).entries.bob
        .write,
    ).toEqual({ effect: "allow", source: CONTENT });
  });

  it("applies a global grant at the closest level even when the user has no resource policy", () => {
    const grants = [
      ...levelGrants(CONTENT, { bob: { read: "allow" } }),
      ...levelGrants(
        GLOBAL_SOURCE,
        { [groupKey("artists")]: { write: "allow" } },
        10,
      ),
    ];
    const membership: Membership = new Map([["bob", new Set(["artists"])]]);

    const result = resolvePermissions(grants, membership, [CONTENT]);

    expect(result.entries.bob.write).toEqual({
      effect: "allow",
      source: GLOBAL_SOURCE,
      viaGroup: "artists",
    });
  });
});

const mockAuth: PermissionAuth = {
  accessToken: "test-access-token",
  idToken: "test-id-token",
  onExpire: vi.fn(),
};

const mockConfig: PermissionConfig = {
  url: "https://api.example.com",
  userInfoUrl: "https://userinfo.example.com",
};

const server = setupServer(
  http.get("https://api.example.com/v1beta/policies/", ({ request }) => {
    const url = new URL(request.url);
    expect(url.searchParams.get("scope")).toBe("inherited");
    const resource = url.searchParams.get("resource") ?? "";
    if (resource.includes("/folder/")) {
      return HttpResponse.json({
        items: [
          {
            id: 1,
            policy:
              'permit(principal == Principal::"bob", action == Action::"storage-service:read", resource == object::"x");',
            principal: { sub: "bob", info: null },
            action: { name: "read", service: "storage-service" },
            resource: { id: "x", type: "object", data: null },
          },
        ],
        page: 1,
        page_size: 1,
      });
    }
    if (resource.includes("/container/")) {
      return HttpResponse.json({
        items: [
          {
            id: 2,
            policy:
              'permit(principal in Group::"artists", action == Action::"storage-service:read", resource == folder::"x");',
            principal: null,
            group: { id: "artists" },
            action: { name: "read", service: "storage-service" },
            resource: { id: "x", type: "folder", data: null },
          },
          {
            id: 3,
            policy:
              'permit(principal in Group::"artists", action == Action::"storage-service:write", resource == folder::"x");',
            principal: null,
            group: { id: "artists" },
            action: { name: "write", service: "storage-service" },
            resource: { id: "x", type: "folder", data: null },
          },
        ],
        page: 1,
        page_size: 2,
      });
    }
    return HttpResponse.json({ items: [], page: 1, page_size: 0 });
  }),
  http.get(
    "https://userinfo.example.com/api/v1alpha/users/:userId/groups",
    ({ params }) => {
      if (params.userId === "bob") {
        return HttpResponse.json({
          items: [{ id: "artists", display_name: "Artists" }],
          next_cursor: null,
        });
      }
      return HttpResponse.json({ items: [], next_cursor: null });
    },
  ),
);

describe("getResolvedPermissions", () => {
  beforeAll(() => server.listen());
  afterEach(() => server.resetHandlers());
  afterAll(() => server.close());

  it("walks ancestors and expands group membership into the user", async () => {
    const resource: ResourceAddress = { address: CONTENT, type: "object" };

    const result = await getResolvedPermissions({
      auth: mockAuth,
      config: mockConfig,
      resource,
    });

    expect(result.entries.bob).toEqual({
      read: { effect: "allow", source: CONTENT },
      write: { effect: "allow", source: MAIN, viaGroup: "artists" },
      edit: { effect: "unset" },
    });
    expect(result.entries[groupKey("artists")]).toEqual({
      read: { effect: "allow", source: MAIN },
      write: { effect: "allow", source: MAIN },
      edit: { effect: "unset" },
    });
  });

  it("skips group expansion when no user info service is configured", async () => {
    const resource: ResourceAddress = { address: CONTENT, type: "object" };

    const result = await getResolvedPermissions({
      auth: mockAuth,
      config: { url: mockConfig.url },
      resource,
    });

    expect(result.entries.bob.write).toEqual({ effect: "unset" });
  });

  it("shows a prefix policy as coming from its parent folder even when it wins over a closer exact policy", async () => {
    server.use(
      http.get("https://api.example.com/v1beta/policies/", ({ request }) => {
        const resource =
          new URL(request.url).searchParams.get("resource") ?? "";
        if (resource.includes("/folder/")) {
          return HttpResponse.json({
            items: [
              {
                id: 1,
                order: 100,
                policy: `permit(principal == Principal::"carol", action == Action::"storage-service:read", resource in folder::"${MAIN}");`,
                principal: { sub: "carol", info: null },
                action: { name: "read", service: "storage-service" },
                resource: null,
                resource_prefix: { id: MAIN, type: "folder", data: null },
              },
              {
                id: 2,
                order: 500,
                policy: `forbid(principal == Principal::"carol", action == Action::"storage-service:read", resource == folder::"${CONTENT}");`,
                principal: { sub: "carol", info: null },
                action: { name: "read", service: "storage-service" },
                resource: { id: CONTENT, type: "folder", data: null },
              },
            ],
            page: 1,
            page_size: 2,
          });
        }
        return HttpResponse.json({ items: [], page: 1, page_size: 0 });
      }),
    );

    const resource: ResourceAddress = { address: CONTENT, type: "object" };
    const result = await getResolvedPermissions({
      auth: mockAuth,
      config: mockConfig,
      resource,
    });

    expect(result.entries.carol.read).toEqual({
      effect: "allow",
      source: MAIN,
      overridden: true,
    });
  });

  it("does not inherit an exact permissions:edit policy from an ancestor folder", async () => {
    server.use(
      http.get("https://api.example.com/v1beta/policies/", ({ request }) => {
        const resource =
          new URL(request.url).searchParams.get("resource") ?? "";
        if (
          resource.includes("/container/") &&
          !resource.includes("/folder/")
        ) {
          return HttpResponse.json({
            items: [
              {
                id: 10,
                policy: `permit(principal == Principal::"bob", action == Action::"permissions:edit", resource == folder::"${MAIN}");`,
                principal: { sub: "bob", info: null },
                action: { name: "edit", service: "permissions" },
                resource: { id: MAIN, type: "folder", data: null },
              },
            ],
            page: 1,
            page_size: 1,
          });
        }
        return HttpResponse.json({ items: [], page: 1, page_size: 0 });
      }),
    );

    const resource: ResourceAddress = { address: CONTENT, type: "object" };
    const result = await getResolvedPermissions({
      auth: mockAuth,
      config: mockConfig,
      resource,
    });

    expect(result.entries.bob.edit).toEqual({ effect: "unset" });
  });

  it("resolves a permissions:edit policy assigned on the selected folder via its resource prefix", async () => {
    server.use(
      http.get("https://api.example.com/v1beta/policies/", ({ request }) => {
        const resource =
          new URL(request.url).searchParams.get("resource") ?? "";
        if (
          resource.includes("/container/") &&
          !resource.includes("/folder/")
        ) {
          return HttpResponse.json({
            items: [
              {
                id: 11,
                policy: `permit(principal == Principal::"bob", action == Action::"permissions:edit", resource in folder::"${MAIN}");`,
                principal: { sub: "bob", info: null },
                action: { name: "edit", service: "permissions" },
                resource: null,
                resource_prefix: { id: MAIN, type: "folder", data: null },
              },
            ],
            page: 1,
            page_size: 1,
          });
        }
        return HttpResponse.json({ items: [], page: 1, page_size: 0 });
      }),
    );

    const resource: ResourceAddress = { address: MAIN, type: "folder" };
    const result = await getResolvedPermissions({
      auth: mockAuth,
      config: mockConfig,
      resource,
    });

    expect(result.entries.bob.edit).toEqual({ effect: "allow", source: MAIN });
  });
});
