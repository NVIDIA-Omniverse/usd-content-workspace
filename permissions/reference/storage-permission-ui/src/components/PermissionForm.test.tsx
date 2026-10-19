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

import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MantineProvider } from "@mantine/core";
import { Notifications, notifications } from "@mantine/notifications";
import { http, HttpResponse } from "msw";
import { setupServer } from "msw/node";
import { PermissionForm } from "./PermissionForm.js";
import type { PermissionAuth } from "../types/PermissionAuth";
import type { PermissionConfig } from "../types/PermissionConfig";
import type { ResourceAddress } from "../types/ResourceAddress";
import type { GroupInfo, UserInfo, UserPage } from "../api/userInfo";

vi.mock("@mantine/notifications", async () => {
  const actual = await vi.importActual("@mantine/notifications");
  return {
    ...actual,
    notifications: {
      show: vi.fn(),
    },
  };
});

// PermissionForm decodes the ID token with jwt-decode, so the fixture has to be
// a well-formed JWT. It is assembled here rather than pasted in as a literal:
// a literal three-segment token in the source trips credential scanners on this
// repository, which is published externally. jwt-decode never verifies the
// signature, so the third segment is a placeholder.
const base64UrlEncode = (value: object) =>
  btoa(JSON.stringify(value))
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/, "");

const mockIdToken = [
  base64UrlEncode({ alg: "HS256", typ: "JWT" }),
  base64UrlEncode({ sub: "1234567890", name: "John Doe", iat: 1516239022 }),
  "not-a-real-signature",
].join(".");

const mockAuth: PermissionAuth = {
  accessToken: "mock-access-token",
  idToken: mockIdToken,
  onExpire: vi.fn(),
};

const mockConfig: PermissionConfig = {
  url: "https://api.example.com",
};

const mockConfigWithUserService: PermissionConfig = {
  url: "https://api.example.com",
  userInfoUrl: "https://userinfo.example.com",
};

const mockResource: ResourceAddress = {
  address: "https://storage.example.com/bucket/file.txt",
  type: "object",
};

const mockServiceMetadata = {
  service: "storage-service",
  id_claim: "sub",
};

const mockUsers: UserInfo[] = [
  {
    id: "user-aaa-111",
    display_name: "Alice Johnson",
    email: "alice@example.com",
    given_name: "Alice",
    family_name: "Johnson",
    username: "alice.johnson",
    active: true,
  },
  {
    id: "user-bbb-222",
    display_name: "Bob Smith",
    email: "bob@example.com",
    given_name: "Bob",
    family_name: "Smith",
    username: "bob.smith",
    active: true,
  },
  {
    id: "user-ccc-333",
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
    id: "group-aaa-111",
    display_name: "Platform Team",
    description: "Platform engineering",
  },
  {
    id: "group-bbb-222",
    display_name: "Design Team",
    description: "Product design",
  },
];

const server = setupServer(
  http.get("https://api.example.com/v1beta/services/storage-service/", () => {
    return HttpResponse.json(mockServiceMetadata);
  }),
  http.post("https://api.example.com/v1beta/authorization/batch/", () => {
    return HttpResponse.json({
      decisions: [
        {
          "permissions:view": { decision: "allow" },
          "permissions:edit": { decision: "allow" },
        },
      ],
    });
  }),
  http.get("https://api.example.com/v1beta/policies/", ({ request }) => {
    const resource = new URL(request.url).searchParams.get("resource");
    if (resource === "NULL") {
      return HttpResponse.json({ items: [], page: 1, page_size: 0 });
    }
    return HttpResponse.json({
      items: [
        {
          id: 1,
          policy: "permit(principal, action, resource);",
          principal: { sub: "user@example.com", info: null },
          action: { name: "read", service: "storage-service" },
          resource: { id: "encoded-resource", type: "object", data: null },
        },
        {
          id: 2,
          policy: "permit(principal, action, resource);",
          principal: { sub: "admin@example.com", info: null },
          action: { name: "read", service: "storage-service" },
          resource: { id: "encoded-resource", type: "object", data: null },
        },
        {
          id: 3,
          policy: "permit(principal, action, resource);",
          principal: { sub: "admin@example.com", info: null },
          action: { name: "write", service: "storage-service" },
          resource: { id: "encoded-resource", type: "object", data: null },
        },
      ],
      page: 1,
      page_size: 3,
    });
  }),
  http.post("https://api.example.com/v1beta/policies/", () => {
    return HttpResponse.json({});
  }),
  http.put("https://api.example.com/v1beta/policies/batch/", () => {
    return HttpResponse.json({});
  }),
  http.delete("https://api.example.com/v1beta/policies/:id", () => {
    return HttpResponse.json({});
  }),
  http.get("https://userinfo.example.com/api/v1alpha/users/me", () => {
    return HttpResponse.json({
      id: "current-user-service-id",
      display_name: "John Doe",
      email: "john@example.com",
    } satisfies UserInfo);
  }),
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
  http.get("https://userinfo.example.com/api/v1alpha/users", ({ request }) => {
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
    } satisfies UserPage);
  }),
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
  http.get("https://userinfo.example.com/api/v1alpha/groups", ({ request }) => {
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
  }),
  http.get(
    "https://userinfo.example.com/api/v1alpha/users/:userId/groups",
    () => HttpResponse.json({ items: [], next_cursor: null }),
  ),
);

async function findEditableRegion() {
  return within(
    await screen.findByRole("region", { name: "Assigned to selection" }),
  );
}

async function waitForLoadersToClear() {
  await waitFor(() =>
    expect(document.querySelectorAll(".mantine-Loader-root")).toHaveLength(0),
  );
}

const TestWrapper = ({ children }: { children: React.ReactNode }) => {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: {
        retry: false,
      },
    },
  });

  return (
    <QueryClientProvider client={queryClient}>
      <MantineProvider env={"test"}>
        <Notifications />
        {children}
      </MantineProvider>
    </QueryClientProvider>
  );
};

describe("PermissionForm", () => {
  beforeAll(() => server.listen());
  afterEach(() => server.resetHandlers());
  afterAll(() => server.close());

  it("renders loading state initially", () => {
    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfig}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    expect(document.querySelector(".mantine-Loader-root")).toBeInTheDocument();
  });

  it("renders permission table when data loads successfully", async () => {
    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfig}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    const editable = await findEditableRegion();
    expect(editable.getByRole("table")).toBeInTheDocument();
    expect(editable.getByText("user@example.com")).toBeInTheDocument();
    expect(editable.getByText("admin@example.com")).toBeInTheDocument();
  });

  it("renders error when permissions fail to load", async () => {
    server.use(
      http.get("https://api.example.com/v1beta/policies/", () => {
        return HttpResponse.json({ error: "Unauthorized" }, { status: 401 });
      }),
    );

    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfig}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    expect(
      await screen.findByText("Failed to load permissions"),
    ).toBeInTheDocument();
  });

  it("renders error when metadata fails to load", async () => {
    server.use(
      http.get(
        "https://api.example.com/v1beta/services/storage-service/",
        () => {
          return HttpResponse.json(
            { error: "Service unavailable" },
            { status: 503 },
          );
        },
      ),
    );

    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfig}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    expect(
      await screen.findByText("Failed to load user identity"),
    ).toBeInTheDocument();
  });

  it("renders error when user access fails to load", async () => {
    server.use(
      http.post("https://api.example.com/v1beta/authorization/batch/", () => {
        return HttpResponse.json({ error: "Forbidden" }, { status: 403 });
      }),
    );

    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfig}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    expect(
      await screen.findByText("Failed to load user access"),
    ).toBeInTheDocument();
  });

  it("renders access denied when user cannot view permissions", async () => {
    server.use(
      http.post("https://api.example.com/v1beta/authorization/batch/", () => {
        return HttpResponse.json({
          decisions: [
            {
              "permissions:view": { decision: "deny" },
              "permissions:edit": { decision: "deny" },
            },
          ],
        });
      }),
    );

    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfig}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    expect(
      await screen.findByText(
        "You're not allowed to view permissions for this resource.",
      ),
    ).toBeInTheDocument();
  });
});

describe("PermissionFormTable", () => {
  beforeAll(() => server.listen());
  afterEach(() => {
    server.resetHandlers();
    vi.mocked(notifications.show).mockClear();
  });
  afterAll(() => server.close());

  it("renders permission table with existing permissions", async () => {
    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfig}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    const editable = await findEditableRegion();
    expect(editable.getByRole("table")).toBeInTheDocument();
    expect(editable.getByText("user@example.com")).toBeInTheDocument();
    expect(editable.getByText("admin@example.com")).toBeInTheDocument();
    expect(editable.getByText("Principal")).toBeInTheDocument();
    expect(editable.getByText("Read")).toBeInTheDocument();
    expect(editable.getByText("Write")).toBeInTheDocument();
    expect(editable.getByText("ACL")).toBeInTheDocument();
    expect(editable.getByLabelText("About the ACL column")).toBeInTheDocument();
  });

  it("cycles permissions through allow, deny and unset states", async () => {
    const user = userEvent.setup();
    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfig}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    await findEditableRegion();

    const userReadCheckbox = screen.getByRole("checkbox", {
      name: "Read is allowed for user@example.com",
    });
    expect(userReadCheckbox).toBeInTheDocument();
    expect(userReadCheckbox).toBeChecked();
    expect(userReadCheckbox).not.toBePartiallyChecked();

    await user.click(userReadCheckbox);
    expect(userReadCheckbox).not.toBeChecked();
    expect(userReadCheckbox).toBePartiallyChecked();

    await user.click(userReadCheckbox);
    expect(userReadCheckbox).not.toBeChecked();
    expect(userReadCheckbox).not.toBePartiallyChecked();

    await user.click(userReadCheckbox);
    expect(userReadCheckbox).toBeChecked();
    expect(userReadCheckbox).not.toBePartiallyChecked();
  });

  it("cycles the ACL permission through allow, deny and unset states", async () => {
    const user = userEvent.setup();
    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfig}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    await findEditableRegion();

    const userAclCheckbox = screen.getByRole("checkbox", {
      name: "ACL is not set for user@example.com",
    });
    expect(userAclCheckbox).toBeInTheDocument();
    expect(userAclCheckbox).not.toBeChecked();
    expect(userAclCheckbox).not.toBePartiallyChecked();

    await user.click(userAclCheckbox);
    expect(userAclCheckbox).toBeChecked();

    await user.click(userAclCheckbox);
    expect(userAclCheckbox).toBePartiallyChecked();

    await user.click(userAclCheckbox);
    expect(userAclCheckbox).not.toBeChecked();
    expect(userAclCheckbox).not.toBePartiallyChecked();
  });

  it("submits a permissions:edit policy when the ACL checkbox is enabled", async () => {
    const user = userEvent.setup();
    let batchPayload: Array<Record<string, unknown>> | null = null;
    server.use(
      http.put(
        "https://api.example.com/v1beta/policies/batch/",
        async ({ request }) => {
          batchPayload = (await request.json()) as Array<
            Record<string, unknown>
          >;
          return HttpResponse.json({});
        },
      ),
    );

    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfig}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    await findEditableRegion();

    const userAclCheckbox = screen.getByRole("checkbox", {
      name: "ACL is not set for user@example.com",
    });
    await user.click(userAclCheckbox);

    const submitButton = screen.getByRole("button", { name: /submit/i });
    await user.click(submitButton);

    await waitFor(() => expect(batchPayload).not.toBeNull());

    expect(batchPayload).toHaveLength(1);
    expect(batchPayload![0].policy).toContain(
      'action == Action::"permissions:edit"',
    );
    expect(batchPayload![0].policy).toContain(
      'principal == Principal::"user@example.com"',
    );
  });

  it("shows containment-scoped policies as read-only rows", async () => {
    server.use(
      http.get("https://api.example.com/v1beta/policies/", ({ request }) => {
        const resource = new URL(request.url).searchParams.get("resource");
        if (resource === "NULL") {
          return HttpResponse.json({ items: [], page: 1, page_size: 0 });
        }
        return HttpResponse.json({
          items: [
            {
              id: 42,
              policy:
                'permit(principal == Principal::"scoped@example.com", action == Action::"storage-service:read", resource in folder::"https://storage.example.com/bucket/");',
              principal: { sub: "scoped@example.com", info: null },
              action: { name: "read", service: "storage-service" },
              resource: null,
            },
          ],
          page: 1,
          page_size: 1,
        });
      }),
    );

    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfig}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    await findEditableRegion();

    const readCheckbox = screen.getByRole("checkbox", {
      name: "Read is allowed for scoped@example.com. Applied by a policy that can't be managed here.",
    });
    expect(readCheckbox).toBeChecked();
    expect(readCheckbox).toBeDisabled();

    expect(
      screen.queryByRole("button", { name: "Remove scoped@example.com" }),
    ).not.toBeInTheDocument();
  });

  it("shows SYSTEM-created ACL policies as read-only", async () => {
    server.use(
      http.get("https://api.example.com/v1beta/policies/", ({ request }) => {
        const resource = new URL(request.url).searchParams.get("resource");
        if (resource === "NULL") {
          return HttpResponse.json({ items: [], page: 1, page_size: 0 });
        }
        return HttpResponse.json({
          items: [
            {
              id: 43,
              created_by: "SYSTEM",
              policy:
                'permit(principal == Principal::"system@example.com", action == Action::"permissions:edit", resource in folder::"https://storage.example.com/bucket/");',
              principal: { sub: "system@example.com", info: null },
              action: { name: "edit", service: "permissions" },
              resource: null,
              resource_prefix: {
                id: "https://storage.example.com/bucket/",
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

    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfig}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    await findEditableRegion();

    const aclCheckbox = screen.getByRole("checkbox", {
      name: "ACL is allowed for system@example.com. Applied by a policy that can't be managed here.",
    });
    expect(aclCheckbox).toBeChecked();
    expect(aclCheckbox).toBeDisabled();

    expect(
      screen.queryByRole("button", { name: "Remove system@example.com" }),
    ).not.toBeInTheDocument();
  });

  it("starts newly added principals in the unset state", async () => {
    const user = userEvent.setup();
    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfig}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    const addInput = await screen.findByPlaceholderText("Add new principal...");
    await user.type(addInput, "newuser@example.com");
    await user.keyboard("{Enter}");

    const readCheckbox = screen.getByRole("checkbox", {
      name: "Read is not set for newuser@example.com",
    });
    const writeCheckbox = screen.getByRole("checkbox", {
      name: "Write is not set for newuser@example.com",
    });
    expect(readCheckbox).not.toBeChecked();
    expect(readCheckbox).not.toBePartiallyChecked();
    expect(writeCheckbox).not.toBeChecked();
    expect(writeCheckbox).not.toBePartiallyChecked();
  });

  it("removes all explicit policies when a permission is unset", async () => {
    const user = userEvent.setup();
    const deletedPolicyIds: number[] = [];
    let batchPayload: unknown[] | null = null;

    server.use(
      http.delete(
        "https://api.example.com/v1beta/policies/:id",
        ({ params }) => {
          deletedPolicyIds.push(Number(params.id));
          return HttpResponse.json({});
        },
      ),
      http.put(
        "https://api.example.com/v1beta/policies/batch/",
        async ({ request }) => {
          batchPayload = (await request.json()) as unknown[];
          return HttpResponse.json({});
        },
      ),
    );

    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfig}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    await findEditableRegion();

    const userReadCheckbox = screen.getByRole("checkbox", {
      name: "Read is allowed for user@example.com",
    });
    await user.click(userReadCheckbox);
    await user.click(userReadCheckbox);
    expect(userReadCheckbox).not.toBeChecked();
    expect(userReadCheckbox).not.toBePartiallyChecked();

    const submitButton = screen.getByRole("button", { name: /submit/i });
    await user.click(submitButton);

    await waitFor(() => {
      expect(batchPayload).not.toBeNull();
    });

    expect(deletedPolicyIds).toContain(1);
    expect(batchPayload).toHaveLength(0);
  });

  it("allows adding new principal", async () => {
    const user = userEvent.setup();
    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfig}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    const addInput = await screen.findByPlaceholderText("Add new principal...");
    expect(addInput).toBeInTheDocument();

    await user.type(addInput, "newuser@example.com");
    await user.keyboard("{Enter}");

    expect(screen.getByText("newuser@example.com")).toBeInTheDocument();
  });

  it("allows adding new principal via plus button", async () => {
    const user = userEvent.setup();
    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfig}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    const addInput = await screen.findByPlaceholderText("Add new principal...");

    await user.type(addInput, "newuser@example.com");

    const addButton = screen.getByRole("button", { name: "Add principal" });
    expect(addButton).toBeInTheDocument();

    await user.click(addButton);

    expect(screen.getByText("newuser@example.com")).toBeInTheDocument();
  });

  it("prevents adding duplicate principals", async () => {
    const user = userEvent.setup();
    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfig}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    const addInput = await screen.findByPlaceholderText("Add new principal...");
    await user.type(addInput, "user@example.com");
    await user.keyboard("{Enter}");

    const editable = await findEditableRegion();
    const userRows = editable.getAllByText("user@example.com");
    expect(userRows).toHaveLength(1);
  });

  it("prevents adding current user as principal", async () => {
    const user = userEvent.setup();
    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfig}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    const addInput = await screen.findByPlaceholderText("Add new principal...");
    await user.type(addInput, "1234567890");
    await user.keyboard("{Enter}");

    expect(
      await screen.findByText("You can't manage your own permissions."),
    ).toBeInTheDocument();
  });

  it("prevents adding current user by user service ID", async () => {
    const user = userEvent.setup();
    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfigWithUserService}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    const addInput = await screen.findByPlaceholderText("Add new principal...");
    await user.type(addInput, "current-user-service-id");
    await user.keyboard("{Enter}");

    expect(
      await screen.findByText("You can't manage your own permissions."),
    ).toBeInTheDocument();
  });

  it("validates that principal does not have invalid characters", async () => {
    const user = userEvent.setup();
    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfig}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    const addInput = await screen.findByPlaceholderText("Add new principal...");
    await user.type(addInput, '"test"');
    await user.keyboard("{Enter}");
    expect(
      await screen.findByText("Principal contains invalid characters."),
    ).toBeInTheDocument();

    await user.clear(addInput);
    await user.type(addInput, "'test'");
    await user.keyboard("{Enter}");
    expect(
      await screen.findByText("Principal contains invalid characters."),
    ).toBeInTheDocument();

    await user.clear(addInput);
    await user.type(addInput, "\\test\\");
    await user.keyboard("{Enter}");
    expect(
      await screen.findByText("Principal contains invalid characters."),
    ).toBeInTheDocument();
  });

  it("validates that principal has maximum length", async () => {
    const user = userEvent.setup();
    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfig}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    const addInput = await screen.findByPlaceholderText("Add new principal...");
    await user.click(addInput);
    await user.paste("1".repeat(1001));
    await user.keyboard("{Enter}");

    expect(
      await screen.findByText("Principal is too long."),
    ).toBeInTheDocument();
  });

  it("allows removing principals", async () => {
    const user = userEvent.setup();
    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfig}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    const editable = await findEditableRegion();

    const removeButton = screen.getByRole("button", {
      name: "Remove user@example.com",
    });
    expect(removeButton).toBeInTheDocument();

    await user.click(removeButton);

    expect(editable.queryByText("user@example.com")).not.toBeInTheDocument();
  });

  it("submits form with updated permissions", async () => {
    const user = userEvent.setup();
    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfig}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    await findEditableRegion();

    const userWriteCheckbox = screen.getByRole("checkbox", {
      name: "Write is not set for user@example.com",
    });
    await user.click(userWriteCheckbox);

    const submitButton = screen.getByRole("button", { name: /submit/i });
    await user.click(submitButton);

    expect((await findEditableRegion()).getByRole("table")).toBeInTheDocument();

    expect(vi.mocked(notifications.show)).toHaveBeenCalledWith({
      title: "Success",
      message: "Permissions updated successfully.",
      color: "green",
    });
  });

  it("resets form when cancel is clicked", async () => {
    const user = userEvent.setup();
    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfig}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    const addInput = await screen.findByPlaceholderText("Add new principal...");
    await user.type(addInput, "newuser@example.com");
    await user.keyboard("{Enter}");

    expect(screen.getByText("newuser@example.com")).toBeInTheDocument();

    const cancelButton = screen.getByRole("button", { name: /cancel/i });
    await user.click(cancelButton);

    expect(screen.queryByText("newuser@example.com")).not.toBeInTheDocument();
  });

  it("shows error when form submission fails", async () => {
    const user = userEvent.setup();
    server.use(
      http.put("https://api.example.com/v1beta/policies/batch/", () => {
        return HttpResponse.json(
          { error: "Validation failed" },
          { status: 400 },
        );
      }),
    );

    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfig}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    await findEditableRegion();

    const submitButton = screen.getByRole("button", { name: /submit/i });
    await user.click(submitButton);

    expect(
      await screen.findByText("Failed to submit permission changes"),
    ).toBeInTheDocument();
  });

  it("disables form when user cannot edit permissions", async () => {
    server.use(
      http.post("https://api.example.com/v1beta/authorization/batch/", () => {
        return HttpResponse.json({
          decisions: [
            {
              "permissions:view": { decision: "allow" },
              "permissions:edit": { decision: "deny" },
            },
          ],
        });
      }),
    );

    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfig}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    await findEditableRegion();

    const checkboxes = screen.getAllByRole("checkbox");
    checkboxes.forEach((checkbox) => {
      expect(checkbox).toBeDisabled();
    });

    expect(
      screen.queryByPlaceholderText("Add new principal..."),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /submit/i }),
    ).not.toBeInTheDocument();
  });

  it("shows a placeholder when the user cannot edit and there is nothing to display", async () => {
    server.use(
      http.post("https://api.example.com/v1beta/authorization/batch/", () => {
        return HttpResponse.json({
          decisions: [
            {
              "permissions:view": { decision: "allow" },
              "permissions:edit": { decision: "deny" },
            },
          ],
        });
      }),
      http.get("https://api.example.com/v1beta/policies/", () => {
        return HttpResponse.json({ items: [], page: 1, page_size: 0 });
      }),
    );

    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfig}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    expect(
      await screen.findByText(
        /There are no permissions to display, and you're not allowed to change them/i,
      ),
    ).toBeInTheDocument();
    expect(screen.queryByRole("table")).not.toBeInTheDocument();
  });

  it("warns each panel with panel-specific text for conditional global policies", async () => {
    server.use(
      http.get("https://api.example.com/v1beta/policies/", ({ request }) => {
        const resource = new URL(request.url).searchParams.get("resource");
        if (resource === "NULL") {
          return HttpResponse.json({
            items: [
              {
                id: 99,
                policy:
                  "permit(principal, action, resource) when { context.mfa };",
                principal: null,
                action: { name: "read", service: "storage-service" },
                resource: null,
              },
            ],
            page: 1,
            page_size: 1,
          });
        }
        return HttpResponse.json({ items: [], page: 1, page_size: 0 });
      }),
    );

    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfig}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    await waitFor(() => {
      expect(
        screen.getByText("Some policies can't be managed here"),
      ).toBeInTheDocument();
    });
    expect(
      screen.getByText(
        "Some policies apply to this resource but can't be displayed or managed in this panel.",
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByText("Some access rules aren't shown here"),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        "Some global rules affect this resource but can't be shown here, so the actual access may differ from what's listed.",
      ),
    ).toBeInTheDocument();
  });

  it("warns only the assigned panel for plain global policies", async () => {
    server.use(
      http.get("https://api.example.com/v1beta/policies/", ({ request }) => {
        const resource = new URL(request.url).searchParams.get("resource");
        if (resource === "NULL") {
          return HttpResponse.json({
            items: [
              {
                id: 99,
                policy: "permit(principal, action, resource);",
                principal: { sub: "someone@example.com", info: null },
                action: { name: "read", service: "storage-service" },
                resource: null,
              },
            ],
            page: 1,
            page_size: 1,
          });
        }
        return HttpResponse.json({ items: [], page: 1, page_size: 0 });
      }),
    );

    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfig}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    await waitFor(() => {
      expect(
        screen.getByText("Some policies can't be managed here"),
      ).toBeInTheDocument();
    });
    expect(
      screen.queryByText("Some access rules aren't shown here"),
    ).not.toBeInTheDocument();
  });

  it("warns the assigned panel about scoped policies it cannot display", async () => {
    server.use(
      http.get("https://api.example.com/v1beta/policies/", ({ request }) => {
        const resource = new URL(request.url).searchParams.get("resource");
        if (resource === "NULL") {
          return HttpResponse.json({ items: [], page: 1, page_size: 0 });
        }
        return HttpResponse.json({
          items: [
            {
              id: 1,
              policy:
                'permit(principal, action, resource) when { principal.department == "engineering" };',
              principal: { sub: "conditional@example.com", info: null },
              action: { name: "read", service: "storage-service" },
              resource: { id: "encoded-resource", type: "object", data: null },
            },
          ],
          page: 1,
          page_size: 1,
        });
      }),
    );

    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfig}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    await waitFor(() => {
      expect(
        screen.getByText("Some policies can't be managed here"),
      ).toBeInTheDocument();
    });
    expect(
      screen.queryByText("Some access rules aren't shown here"),
    ).not.toBeInTheDocument();
  });

  it("warns in both panels when global policies cannot be loaded", async () => {
    server.use(
      http.get("https://api.example.com/v1beta/policies/", ({ request }) => {
        const resource = new URL(request.url).searchParams.get("resource");
        if (resource === "NULL") {
          return new HttpResponse(null, { status: 500 });
        }
        return HttpResponse.json({ items: [], page: 1, page_size: 0 });
      }),
    );

    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfig}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    await waitFor(() => {
      expect(
        screen.getAllByText("Global policies couldn't be checked"),
      ).toHaveLength(2);
    });
    expect(
      screen.getAllByText(
        "Global policies that may apply to this resource couldn't be loaded, so some access rules might not be shown here.",
      ),
    ).toHaveLength(2);
  });

  it("toggles each panel's warning description independently without hiding it", async () => {
    const user = userEvent.setup();
    server.use(
      http.get("https://api.example.com/v1beta/policies/", ({ request }) => {
        const resource = new URL(request.url).searchParams.get("resource");
        if (resource === "NULL") {
          return HttpResponse.json({
            items: [
              {
                id: 99,
                policy:
                  "permit(principal, action, resource) when { context.mfa };",
                principal: null,
                action: { name: "read", service: "storage-service" },
                resource: null,
              },
            ],
            page: 1,
            page_size: 1,
          });
        }
        return HttpResponse.json({ items: [], page: 1, page_size: 0 });
      }),
    );

    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfig}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    const assignedDescription =
      "Some policies apply to this resource but can't be displayed or managed in this panel.";
    const resolvedDescription =
      "Some global rules affect this resource but can't be shown here, so the actual access may differ from what's listed.";

    await waitFor(() => {
      expect(
        screen.getByText("Some policies can't be managed here"),
      ).toBeInTheDocument();
    });
    expect(screen.getByText(assignedDescription)).toBeInTheDocument();
    expect(screen.getByText(resolvedDescription)).toBeInTheDocument();

    const toggles = screen.getAllByRole("button", {
      name: "Toggle details",
    });
    expect(toggles).toHaveLength(2);
    await user.click(toggles[0]);

    expect(screen.queryByText(assignedDescription)).not.toBeInTheDocument();
    expect(screen.getByText(resolvedDescription)).toBeInTheDocument();

    await user.click(toggles[0]);
    expect(screen.getByText(assignedDescription)).toBeInTheDocument();
  });

  it("does not warn about global policies when they are all supported", async () => {
    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfig}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    await findEditableRegion();

    expect(
      screen.queryByText("Some policies can't be managed here"),
    ).not.toBeInTheDocument();
  });

  it("disables current user's permissions from being edited", async () => {
    server.use(
      http.get("https://api.example.com/v1beta/policies/", () => {
        return HttpResponse.json({
          items: [
            {
              id: 1,
              policy: "permit(principal, action, resource);",
              principal: { sub: "1234567890", info: null },
              action: { name: "read", service: "storage-service" },
              resource: { id: "encoded-resource", type: "object", data: null },
            },
          ],
          page: 1,
          page_size: 1,
        });
      }),
    );

    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfig}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    await findEditableRegion();

    const readCheckbox = screen.getByRole("checkbox", {
      name: "Read is allowed for 1234567890",
    });
    const writeCheckbox = screen.getByRole("checkbox", {
      name: "Write is not set for 1234567890",
    });

    expect(readCheckbox).toBeDisabled();
    expect(writeCheckbox).toBeDisabled();

    expect(
      screen.queryByRole("button", { name: "Remove 1234567890" }),
    ).not.toBeInTheDocument();
  });

  it("disables current user's permissions when matched by user service ID", async () => {
    server.use(
      http.get("https://api.example.com/v1beta/policies/", () => {
        return HttpResponse.json({
          items: [
            {
              id: 1,
              policy: "permit(principal, action, resource);",
              principal: { sub: "current-user-service-id", info: null },
              action: { name: "read", service: "storage-service" },
              resource: { id: "encoded-resource", type: "object", data: null },
            },
          ],
          page: 1,
          page_size: 1,
        });
      }),
    );

    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfigWithUserService}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    await findEditableRegion();

    const readCheckbox = screen.getByRole("checkbox", {
      name: "Read is allowed for current-user-service-id",
    });
    const writeCheckbox = screen.getByRole("checkbox", {
      name: "Write is not set for current-user-service-id",
    });

    expect(readCheckbox).toBeDisabled();
    expect(writeCheckbox).toBeDisabled();

    expect(
      screen.queryByRole("button", { name: "Remove current-user-service-id" }),
    ).not.toBeInTheDocument();
  });
});

describe("PermissionFormTable with user info service", () => {
  beforeAll(() => server.listen());
  afterEach(() => {
    server.resetHandlers();
    vi.mocked(notifications.show).mockClear();
  });
  afterAll(() => server.close());

  it("resolves principal IDs to display names when user service is configured", async () => {
    server.use(
      http.get("https://api.example.com/v1beta/policies/", () => {
        return HttpResponse.json({
          items: [
            {
              id: 1,
              policy: "permit(principal, action, resource);",
              principal: { sub: "user-aaa-111", info: null },
              action: { name: "read", service: "storage-service" },
              resource: { id: "encoded-resource", type: "object", data: null },
            },
          ],
          page: 1,
          page_size: 1,
        });
      }),
    );

    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfigWithUserService}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    const editable = await findEditableRegion();
    expect(await editable.findByText("Alice Johnson")).toBeInTheDocument();
    expect(editable.getByText("alice@example.com")).toBeInTheDocument();
  });

  it("shows raw principal ID when user cannot be resolved", async () => {
    server.use(
      http.get("https://api.example.com/v1beta/policies/", () => {
        return HttpResponse.json({
          items: [
            {
              id: 1,
              policy: "permit(principal, action, resource);",
              principal: { sub: "unknown-user-id", info: null },
              action: { name: "read", service: "storage-service" },
              resource: { id: "encoded-resource", type: "object", data: null },
            },
          ],
          page: 1,
          page_size: 1,
        });
      }),
    );

    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfigWithUserService}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    const editable = await findEditableRegion();
    expect(await editable.findByText("unknown-user-id")).toBeInTheDocument();
  });

  it("shows raw principal ID when user service is not configured", async () => {
    server.use(
      http.get("https://api.example.com/v1beta/policies/", () => {
        return HttpResponse.json({
          items: [
            {
              id: 1,
              policy: "permit(principal, action, resource);",
              principal: { sub: "user-aaa-111", info: null },
              action: { name: "read", service: "storage-service" },
              resource: { id: "encoded-resource", type: "object", data: null },
            },
          ],
          page: 1,
          page_size: 1,
        });
      }),
    );

    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfig}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    const editable = await findEditableRegion();
    expect(await editable.findByText("user-aaa-111")).toBeInTheDocument();
  });

  it("shows autocomplete dropdown with user results when typing", async () => {
    const user = userEvent.setup();
    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfigWithUserService}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    const addInput = await screen.findByPlaceholderText("Add new principal...");
    await user.type(addInput, "Alice");

    expect(await screen.findByText("Alice Johnson")).toBeInTheDocument();
    expect(screen.getByText("alice@example.com")).toBeInTheDocument();
  });

  it("adds principal by user ID when autocomplete option is selected", async () => {
    const user = userEvent.setup();
    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfigWithUserService}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    const addInput = await screen.findByPlaceholderText("Add new principal...");
    await user.type(addInput, "Bob");

    const option = await screen.findByText("Bob Smith");
    await user.click(option);

    await waitFor(() => {
      expect(screen.getByText("Bob Smith")).toBeInTheDocument();
    });
    expect(screen.getByText("bob@example.com")).toBeInTheDocument();
  });

  it("shows 'No users found' for non-matching search", async () => {
    const user = userEvent.setup();
    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfigWithUserService}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    const addInput = await screen.findByPlaceholderText("Add new principal...");
    await user.type(addInput, "nonexistentuser");

    expect(await screen.findByText("No users found")).toBeInTheDocument();
  });

  it("allows typing a raw principal ID with Enter when autocomplete is enabled", async () => {
    const user = userEvent.setup();
    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfigWithUserService}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    const addInput = await screen.findByPlaceholderText("Add new principal...");
    await user.type(addInput, "custom-raw-id");
    await user.keyboard("{Enter}");

    expect(await screen.findByText("custom-raw-id")).toBeInTheDocument();
  });

  it("shows multiple matching users in autocomplete dropdown", async () => {
    const user = userEvent.setup();
    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfigWithUserService}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    const addInput = await screen.findByPlaceholderText("Add new principal...");
    await user.type(addInput, "o");

    expect(await screen.findByText("Alice Johnson")).toBeInTheDocument();
    expect(screen.getByText("Bob Smith")).toBeInTheDocument();
    expect(screen.getByText("Charlie Brown")).toBeInTheDocument();
  });

  it("resolves group IDs to display names for group-scoped policies", async () => {
    server.use(
      http.get("https://api.example.com/v1beta/policies/", () => {
        return HttpResponse.json({
          items: [
            {
              id: 1,
              policy:
                'permit(principal in Group::"group-aaa-111", action, resource);',
              principal: null,
              group: { id: "group-aaa-111" },
              action: { name: "read", service: "storage-service" },
              resource: { id: "encoded-resource", type: "object", data: null },
            },
          ],
          page: 1,
          page_size: 1,
        });
      }),
    );

    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfigWithUserService}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    const editable = await findEditableRegion();
    expect(await editable.findByText("Platform Team")).toBeInTheDocument();
    expect(
      screen.getByRole("checkbox", {
        name: "Read is allowed for Platform Team",
      }),
    ).toBeInTheDocument();
  });

  it("shows raw group ID when the group cannot be resolved", async () => {
    server.use(
      http.get("https://api.example.com/v1beta/policies/", () => {
        return HttpResponse.json({
          items: [
            {
              id: 1,
              policy:
                'permit(principal in Group::"unknown-group", action, resource);',
              principal: null,
              group: { id: "unknown-group" },
              action: { name: "read", service: "storage-service" },
              resource: { id: "encoded-resource", type: "object", data: null },
            },
          ],
          page: 1,
          page_size: 1,
        });
      }),
    );

    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfigWithUserService}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    const editable = await findEditableRegion();
    expect(await editable.findByText("unknown-group")).toBeInTheDocument();
  });

  it("adds a group principal via the type selector and autocomplete", async () => {
    const user = userEvent.setup();
    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfigWithUserService}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    await screen.findByPlaceholderText("Add new principal...");
    await waitForLoadersToClear();

    await user.click(screen.getByLabelText("Principal type: User"));
    await user.click(await screen.findByRole("menuitem", { name: "Group" }));

    const addInput = screen.getByPlaceholderText("Add new principal...");
    await user.type(addInput, "Platform");

    const option = await screen.findByText("Platform Team");
    await user.click(option);

    expect(
      await screen.findByRole("checkbox", {
        name: "Read is not set for Platform Team",
      }),
    ).toBeInTheDocument();
  });

  it("submits a group-scoped policy with the group head", async () => {
    const user = userEvent.setup();
    let batchPayload: Array<Record<string, unknown>> | null = null;

    server.use(
      http.put(
        "https://api.example.com/v1beta/policies/batch/",
        async ({ request }) => {
          batchPayload = (await request.json()) as Array<
            Record<string, unknown>
          >;
          return HttpResponse.json({});
        },
      ),
    );

    render(
      <TestWrapper>
        <PermissionForm
          auth={mockAuth}
          config={mockConfigWithUserService}
          resource={mockResource}
        />
      </TestWrapper>,
    );

    await screen.findByPlaceholderText("Add new principal...");
    await waitForLoadersToClear();

    await user.click(screen.getByLabelText("Principal type: User"));
    await user.click(await screen.findByRole("menuitem", { name: "Group" }));

    const addInput = screen.getByPlaceholderText("Add new principal...");
    await user.type(addInput, "group-ccc-333");
    await user.keyboard("{Enter}");

    const readCheckbox = await screen.findByRole("checkbox", {
      name: "Read is not set for group-ccc-333",
    });
    await user.click(readCheckbox);

    const submitButton = screen.getByRole("button", { name: /submit/i });
    await user.click(submitButton);

    await waitFor(() => {
      expect(batchPayload).not.toBeNull();
    });

    expect(batchPayload).toHaveLength(1);
    expect(batchPayload![0].group).toBe("group-ccc-333");
    expect(batchPayload![0].principal).toBeUndefined();
    expect(batchPayload![0].policy).toContain(
      'principal in Group::"group-ccc-333"',
    );
  });
});
