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
  fetchAllUserGroups,
  fetchCurrentUser,
  fetchGroup,
  fetchGroups,
  fetchUser,
  fetchUserGroups,
  fetchUsers,
  formatGroupName,
  formatUserLabel,
  formatUserName,
  type GroupInfo,
  type UserInfo,
} from "./userInfo.js";
import type { PermissionAuth } from "../types/PermissionAuth";
import type { PermissionConfig } from "../types/PermissionConfig";

const mockAuth: PermissionAuth = {
  accessToken: "test-access-token",
  idToken: "test-id-token",
  onExpire: vi.fn(),
};

const mockConfig: PermissionConfig = {
  url: "https://api.example.com",
  userInfoUrl: "https://userinfo.example.com",
};

const mockUsers: UserInfo[] = [
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

const mockGroups: GroupInfo[] = [
  {
    id: "group-123",
    display_name: "Platform Team",
    description: "Platform engineering",
  },
  {
    id: "group-456",
    display_name: "Design Team",
    description: "Product design",
  },
];

const server = setupServer();

beforeEach(() => {
  server.listen();
  vi.clearAllMocks();
});

afterEach(() => {
  server.resetHandlers();
});

describe("fetchCurrentUser", () => {
  it("should fetch the current user", async () => {
    server.use(
      http.get("https://userinfo.example.com/api/v1alpha/users/me", () => {
        return HttpResponse.json(mockUsers[0]);
      }),
    );

    const result = await fetchCurrentUser({
      auth: mockAuth,
      config: mockConfig,
    });

    expect(result.id).toBe("user-123");
    expect(result.display_name).toBe("Alice Johnson");
  });

  it("should send authorization header", async () => {
    server.use(
      http.get(
        "https://userinfo.example.com/api/v1alpha/users/me",
        ({ request }) => {
          expect(request.headers.get("Authorization")).toBe(
            "Bearer test-access-token",
          );
          return HttpResponse.json(mockUsers[0]);
        },
      ),
    );

    await fetchCurrentUser({ auth: mockAuth, config: mockConfig });
  });

  it("should call onExpire on 401", async () => {
    server.use(
      http.get("https://userinfo.example.com/api/v1alpha/users/me", () => {
        return new HttpResponse("Unauthorized", { status: 401 });
      }),
    );

    await expect(
      fetchCurrentUser({ auth: mockAuth, config: mockConfig }),
    ).rejects.toThrow("Failed to fetch current user: HTTP401");
    expect(mockAuth.onExpire).toHaveBeenCalled();
  });

  it("should throw on server error", async () => {
    server.use(
      http.get("https://userinfo.example.com/api/v1alpha/users/me", () => {
        return new HttpResponse("Internal Server Error", { status: 500 });
      }),
    );

    await expect(
      fetchCurrentUser({ auth: mockAuth, config: mockConfig }),
    ).rejects.toThrow("Failed to fetch current user: HTTP500");
  });
});

describe("fetchUser", () => {
  it("should fetch a user by ID", async () => {
    server.use(
      http.get(
        "https://userinfo.example.com/api/v1alpha/users/:userId",
        ({ params }) => {
          const user = mockUsers.find((u) => u.id === params.userId);
          if (user) return HttpResponse.json(user);
          return new HttpResponse(
            JSON.stringify({ error: "not_found", message: "User not found" }),
            { status: 404 },
          );
        },
      ),
    );

    const result = await fetchUser({
      auth: mockAuth,
      config: mockConfig,
      userId: "user-123",
    });

    expect(result.id).toBe("user-123");
    expect(result.display_name).toBe("Alice Johnson");
    expect(result.email).toBe("alice@example.com");
  });

  it("should send authorization header", async () => {
    server.use(
      http.get(
        "https://userinfo.example.com/api/v1alpha/users/:userId",
        ({ request }) => {
          expect(request.headers.get("Authorization")).toBe(
            "Bearer test-access-token",
          );
          return HttpResponse.json(mockUsers[0]);
        },
      ),
    );

    await fetchUser({
      auth: mockAuth,
      config: mockConfig,
      userId: "user-123",
    });
  });

  it("should throw on 404", async () => {
    server.use(
      http.get("https://userinfo.example.com/api/v1alpha/users/:userId", () => {
        return new HttpResponse("Not found", { status: 404 });
      }),
    );

    await expect(
      fetchUser({
        auth: mockAuth,
        config: mockConfig,
        userId: "nonexistent",
      }),
    ).rejects.toThrow("Failed to fetch user: HTTP404");
  });

  it("should call onExpire on 401", async () => {
    server.use(
      http.get("https://userinfo.example.com/api/v1alpha/users/:userId", () => {
        return new HttpResponse("Unauthorized", { status: 401 });
      }),
    );

    await expect(
      fetchUser({
        auth: mockAuth,
        config: mockConfig,
        userId: "user-123",
      }),
    ).rejects.toThrow("Failed to fetch user: HTTP401");
    expect(mockAuth.onExpire).toHaveBeenCalled();
  });

  it("should encode user ID in URL path", async () => {
    server.use(
      http.get(
        "https://userinfo.example.com/api/v1alpha/users/:userId",
        ({ params }) => {
          expect(params.userId).toBe("user/with spaces");
          return HttpResponse.json(mockUsers[0]);
        },
      ),
    );

    await fetchUser({
      auth: mockAuth,
      config: mockConfig,
      userId: "user/with spaces",
    });
  });
});

describe("fetchUsers", () => {
  it("should fetch users with display name filter", async () => {
    server.use(
      http.get(
        "https://userinfo.example.com/api/v1alpha/users",
        ({ request }) => {
          const url = new URL(request.url);
          const displayName = url.searchParams.get("display_name");

          let filtered = [...mockUsers];
          if (displayName) {
            const lower = displayName.toLowerCase();
            filtered = filtered.filter((u) =>
              u.display_name?.toLowerCase().includes(lower),
            );
          }

          return HttpResponse.json({
            items: filtered,
            next_cursor: null,
          });
        },
      ),
    );

    const result = await fetchUsers({
      auth: mockAuth,
      config: mockConfig,
      displayName: "Alice",
    });

    expect(result.items).toHaveLength(1);
    expect(result.items[0].display_name).toBe("Alice Johnson");
  });

  it("should fetch all users without filter", async () => {
    server.use(
      http.get("https://userinfo.example.com/api/v1alpha/users", () => {
        return HttpResponse.json({
          items: mockUsers,
          next_cursor: null,
        });
      }),
    );

    const result = await fetchUsers({
      auth: mockAuth,
      config: mockConfig,
    });

    expect(result.items).toHaveLength(3);
  });

  it("should pass pagination parameters", async () => {
    server.use(
      http.get(
        "https://userinfo.example.com/api/v1alpha/users",
        ({ request }) => {
          const url = new URL(request.url);
          expect(url.searchParams.get("cursor")).toBe("page2");
          expect(url.searchParams.get("limit")).toBe("10");

          return HttpResponse.json({
            items: [mockUsers[2]],
            next_cursor: null,
          });
        },
      ),
    );

    const result = await fetchUsers({
      auth: mockAuth,
      config: mockConfig,
      cursor: "page2",
      limit: 10,
    });

    expect(result.items).toHaveLength(1);
  });

  it("should send authorization header", async () => {
    server.use(
      http.get(
        "https://userinfo.example.com/api/v1alpha/users",
        ({ request }) => {
          expect(request.headers.get("Authorization")).toBe(
            "Bearer test-access-token",
          );
          return HttpResponse.json({ items: [], next_cursor: null });
        },
      ),
    );

    await fetchUsers({ auth: mockAuth, config: mockConfig });
  });

  it("should throw on server error", async () => {
    server.use(
      http.get("https://userinfo.example.com/api/v1alpha/users", () => {
        return new HttpResponse("Internal Server Error", { status: 500 });
      }),
    );

    await expect(
      fetchUsers({ auth: mockAuth, config: mockConfig }),
    ).rejects.toThrow("Failed to fetch users: HTTP500");
  });

  it("should call onExpire on 401", async () => {
    server.use(
      http.get("https://userinfo.example.com/api/v1alpha/users", () => {
        return new HttpResponse("Unauthorized", { status: 401 });
      }),
    );

    await expect(
      fetchUsers({ auth: mockAuth, config: mockConfig }),
    ).rejects.toThrow("Failed to fetch users: HTTP401");
    expect(mockAuth.onExpire).toHaveBeenCalled();
  });

  it("should return next_cursor for pagination", async () => {
    server.use(
      http.get("https://userinfo.example.com/api/v1alpha/users", () => {
        return HttpResponse.json({
          items: [mockUsers[0]],
          next_cursor: "next-page-token",
        });
      }),
    );

    const result = await fetchUsers({
      auth: mockAuth,
      config: mockConfig,
      limit: 1,
    });

    expect(result.next_cursor).toBe("next-page-token");
  });
});

describe("formatUserName", () => {
  it("should return display_name when available", () => {
    expect(formatUserName(mockUsers[0])).toBe("Alice Johnson");
  });

  it("should fall back to email when display_name is missing", () => {
    expect(formatUserName({ id: "user-1", email: "test@example.com" })).toBe(
      "test@example.com",
    );
  });

  it("should fall back to id when both display_name and email are missing", () => {
    expect(formatUserName({ id: "user-1" })).toBe("user-1");
  });
});

describe("formatUserLabel", () => {
  it("should return name and email", () => {
    expect(formatUserLabel(mockUsers[0])).toBe(
      "Alice Johnson (alice@example.com)",
    );
  });

  it("should return id and email when display_name is missing", () => {
    expect(formatUserLabel({ id: "user-1", email: "test@example.com" })).toBe(
      "user-1 (test@example.com)",
    );
  });

  it("should return display_name only when email is missing", () => {
    expect(formatUserLabel({ id: "user-1", display_name: "Test User" })).toBe(
      "Test User",
    );
  });

  it("should return id when both display_name and email are missing", () => {
    expect(formatUserLabel({ id: "user-1" })).toBe("user-1");
  });
});

describe("fetchGroup", () => {
  it("should fetch a group by ID", async () => {
    server.use(
      http.get(
        "https://userinfo.example.com/api/v1alpha/groups/:groupId",
        ({ params }) => {
          const group = mockGroups.find((g) => g.id === params.groupId);
          if (group) return HttpResponse.json(group);
          return new HttpResponse(
            JSON.stringify({ error: "not_found", message: "Group not found" }),
            { status: 404 },
          );
        },
      ),
    );

    const result = await fetchGroup({
      auth: mockAuth,
      config: mockConfig,
      groupId: "group-123",
    });

    expect(result.id).toBe("group-123");
    expect(result.display_name).toBe("Platform Team");
  });

  it("should send authorization header", async () => {
    server.use(
      http.get(
        "https://userinfo.example.com/api/v1alpha/groups/:groupId",
        ({ request }) => {
          expect(request.headers.get("Authorization")).toBe(
            "Bearer test-access-token",
          );
          return HttpResponse.json(mockGroups[0]);
        },
      ),
    );

    await fetchGroup({
      auth: mockAuth,
      config: mockConfig,
      groupId: "group-123",
    });
  });

  it("should encode group ID in URL path", async () => {
    server.use(
      http.get(
        "https://userinfo.example.com/api/v1alpha/groups/:groupId",
        ({ params }) => {
          expect(params.groupId).toBe("group/with spaces");
          return HttpResponse.json(mockGroups[0]);
        },
      ),
    );

    await fetchGroup({
      auth: mockAuth,
      config: mockConfig,
      groupId: "group/with spaces",
    });
  });

  it("should throw on 404", async () => {
    server.use(
      http.get(
        "https://userinfo.example.com/api/v1alpha/groups/:groupId",
        () => {
          return new HttpResponse("Not found", { status: 404 });
        },
      ),
    );

    await expect(
      fetchGroup({
        auth: mockAuth,
        config: mockConfig,
        groupId: "nonexistent",
      }),
    ).rejects.toThrow("Failed to fetch group: HTTP404");
  });

  it("should call onExpire on 401", async () => {
    server.use(
      http.get(
        "https://userinfo.example.com/api/v1alpha/groups/:groupId",
        () => {
          return new HttpResponse("Unauthorized", { status: 401 });
        },
      ),
    );

    await expect(
      fetchGroup({
        auth: mockAuth,
        config: mockConfig,
        groupId: "group-123",
      }),
    ).rejects.toThrow("Failed to fetch group: HTTP401");
    expect(mockAuth.onExpire).toHaveBeenCalled();
  });
});

describe("fetchGroups", () => {
  it("should fetch groups with display name filter", async () => {
    server.use(
      http.get(
        "https://userinfo.example.com/api/v1alpha/groups",
        ({ request }) => {
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
          });
        },
      ),
    );

    const result = await fetchGroups({
      auth: mockAuth,
      config: mockConfig,
      displayName: "Platform",
    });

    expect(result.items).toHaveLength(1);
    expect(result.items[0].display_name).toBe("Platform Team");
  });

  it("should pass pagination parameters", async () => {
    server.use(
      http.get(
        "https://userinfo.example.com/api/v1alpha/groups",
        ({ request }) => {
          const url = new URL(request.url);
          expect(url.searchParams.get("cursor")).toBe("page2");
          expect(url.searchParams.get("limit")).toBe("10");

          return HttpResponse.json({
            items: [mockGroups[1]],
            next_cursor: null,
          });
        },
      ),
    );

    const result = await fetchGroups({
      auth: mockAuth,
      config: mockConfig,
      cursor: "page2",
      limit: 10,
    });

    expect(result.items).toHaveLength(1);
  });

  it("should call onExpire on 401", async () => {
    server.use(
      http.get("https://userinfo.example.com/api/v1alpha/groups", () => {
        return new HttpResponse("Unauthorized", { status: 401 });
      }),
    );

    await expect(
      fetchGroups({ auth: mockAuth, config: mockConfig }),
    ).rejects.toThrow("Failed to fetch groups: HTTP401");
    expect(mockAuth.onExpire).toHaveBeenCalled();
  });
});

describe("fetchUserGroups", () => {
  it("should request the user's groups endpoint and pass pagination params", async () => {
    server.use(
      http.get(
        "https://userinfo.example.com/api/v1alpha/users/:userId/groups",
        ({ request, params }) => {
          expect(params.userId).toBe("user-123");
          const url = new URL(request.url);
          expect(url.searchParams.get("cursor")).toBe("page2");
          expect(url.searchParams.get("limit")).toBe("10");
          expect(request.headers.get("Authorization")).toBe(
            "Bearer test-access-token",
          );
          return HttpResponse.json({
            items: [mockGroups[1]],
            next_cursor: null,
          });
        },
      ),
    );

    const result = await fetchUserGroups({
      auth: mockAuth,
      config: mockConfig,
      userId: "user-123",
      cursor: "page2",
      limit: 10,
    });

    expect(result.items).toHaveLength(1);
    expect(result.items[0].id).toBe("group-456");
  });

  it("should encode the user ID in the URL path", async () => {
    server.use(
      http.get(
        "https://userinfo.example.com/api/v1alpha/users/:userId/groups",
        ({ params }) => {
          expect(params.userId).toBe("user/with spaces");
          return HttpResponse.json({ items: [], next_cursor: null });
        },
      ),
    );

    await fetchUserGroups({
      auth: mockAuth,
      config: mockConfig,
      userId: "user/with spaces",
    });
  });

  it("should call onExpire on 401", async () => {
    server.use(
      http.get(
        "https://userinfo.example.com/api/v1alpha/users/:userId/groups",
        () => new HttpResponse("Unauthorized", { status: 401 }),
      ),
    );

    await expect(
      fetchUserGroups({
        auth: mockAuth,
        config: mockConfig,
        userId: "user-123",
      }),
    ).rejects.toThrow("Failed to fetch user groups: HTTP401");
    expect(mockAuth.onExpire).toHaveBeenCalled();
  });
});

describe("fetchAllUserGroups", () => {
  it("should accumulate groups across cursor pages until next_cursor is null", async () => {
    const requestedCursors: Array<string | null> = [];
    server.use(
      http.get(
        "https://userinfo.example.com/api/v1alpha/users/:userId/groups",
        ({ request }) => {
          const url = new URL(request.url);
          const cursor = url.searchParams.get("cursor");
          requestedCursors.push(cursor);
          if (!cursor) {
            return HttpResponse.json({
              items: [mockGroups[0]],
              next_cursor: "page-2",
            });
          }
          return HttpResponse.json({
            items: [mockGroups[1]],
            next_cursor: null,
          });
        },
      ),
    );

    const groups = await fetchAllUserGroups({
      auth: mockAuth,
      config: mockConfig,
      userId: "user-123",
    });

    expect(groups).toHaveLength(2);
    expect(groups.map((g) => g.id)).toEqual(["group-123", "group-456"]);
    expect(requestedCursors).toContain("page-2");
  });

  it("should return a single page when there is no next cursor", async () => {
    server.use(
      http.get(
        "https://userinfo.example.com/api/v1alpha/users/:userId/groups",
        () =>
          HttpResponse.json({
            items: mockGroups,
            next_cursor: null,
          }),
      ),
    );

    const groups = await fetchAllUserGroups({
      auth: mockAuth,
      config: mockConfig,
      userId: "user-123",
    });

    expect(groups).toHaveLength(2);
  });
});

describe("formatGroupName", () => {
  it("should return display_name when available", () => {
    expect(formatGroupName(mockGroups[0])).toBe("Platform Team");
  });

  it("should fall back to id when display_name is missing", () => {
    expect(formatGroupName({ id: "group-1" })).toBe("group-1");
  });
});
