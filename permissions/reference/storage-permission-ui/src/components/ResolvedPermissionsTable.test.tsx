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

import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MantineProvider } from "@mantine/core";
import { ResolvedPermissionsTable } from "./ResolvedPermissionsTable.js";
import { ALL_USERS, groupKey, type PermissionEffect } from "../api/policies.js";
import type { ResolvedPermissions } from "../api/resolved.js";
import type { PermissionAuth } from "../types/PermissionAuth";
import type { PermissionConfig } from "../types/PermissionConfig";
import type { ResourceAddress } from "../types/ResourceAddress";

const HOST = "https://exampleaccount.blob.core.windows.net";
const CONTENT = `${HOST}/container/folder/`;
const MAIN = `${HOST}/container/`;
const ROOT = `${HOST}/`;

const mockAuth: PermissionAuth = {
  accessToken: "token",
  idToken: "id-token",
  onExpire: () => {},
};

const mockConfig: PermissionConfig = {
  url: "https://api.example.com",
};

const resource: ResourceAddress = { address: CONTENT, type: "object" };

function permission(
  effect: PermissionEffect,
  source?: string,
  viaGroup?: string,
) {
  return { effect, source, viaGroup };
}

const data: ResolvedPermissions = {
  entries: {
    [ALL_USERS]: {
      read: permission("allow", ROOT),
      write: permission("deny", ROOT),
      edit: permission("unset"),
    },
    bob: {
      read: permission("allow", CONTENT),
      write: permission("allow", MAIN, "artists"),
      edit: permission("allow", CONTENT),
    },
    [groupKey("artists")]: {
      read: permission("allow", MAIN),
      write: permission("deny", ROOT),
      edit: permission("unset"),
    },
  },
};

function renderTable() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <MantineProvider>
        <ResolvedPermissionsTable
          auth={mockAuth}
          config={mockConfig}
          data={data}
          resource={resource}
        />
      </MantineProvider>
    </QueryClientProvider>,
  );
}

describe("ResolvedPermissionsTable", () => {
  it("renders a row for each principal with All Users first", () => {
    renderTable();

    const rows = screen.getAllByRole("row").slice(1);
    expect(rows[0]).toHaveTextContent("All Users");
    expect(rows[1]).toHaveTextContent("bob");
    expect(rows[2]).toHaveTextContent("artists");
  });

  it("renders effects read-only with the correct state", () => {
    renderTable();

    const bobRead = screen.getByRole("checkbox", {
      name: "Resolved read for bob: allowed. Applied to the current selection",
    });
    expect(bobRead).toBeChecked();
    expect(bobRead).toBeDisabled();

    const bobWrite = screen.getByRole("checkbox", {
      name: `Resolved write for bob: allowed. Inherited from ${MAIN} via group artists`,
    });
    expect(bobWrite).toBeChecked();

    const allUsersWrite = screen.getByRole("checkbox", {
      name: `Resolved write for All Users: denied. Inherited from ${ROOT}`,
    });
    expect(allUsersWrite).toBePartiallyChecked();
    expect(allUsersWrite).toBeDisabled();

    const bobAcl = screen.getByRole("checkbox", {
      name: "Resolved ACL for bob: allowed. Applied to the current selection",
    });
    expect(bobAcl).toBeChecked();
    expect(bobAcl).toBeDisabled();

    const allUsersAcl = screen.getByRole("checkbox", {
      name: "Resolved ACL for All Users: denied. Denied by default, as no policy applies",
    });
    expect(allUsersAcl).not.toBeChecked();
    expect(allUsersAcl).toBePartiallyChecked();
    expect(allUsersAcl).toBeDisabled();
  });

  it("shows an unset resolved permission as denied with an explanatory tooltip", async () => {
    const user = userEvent.setup();
    renderTable();

    const allUsersAcl = screen.getByRole("checkbox", {
      name: "Resolved ACL for All Users: denied. Denied by default, as no policy applies",
    });
    expect(allUsersAcl).toBePartiallyChecked();

    const tooltipTarget = allUsersAcl.closest("td")!.firstElementChild!;
    await user.hover(tooltipTarget);

    expect(
      await screen.findByText("Denied by default, as no policy applies"),
    ).toBeInTheDocument();
  });

  it("disables every effect checkbox", () => {
    renderTable();

    screen
      .getAllByRole("checkbox")
      .forEach((checkbox) => expect(checkbox).toBeDisabled());
  });

  it("renders nothing when there are no resolved entries", () => {
    const queryClient = new QueryClient();
    const { container } = render(
      <QueryClientProvider client={queryClient}>
        <MantineProvider>
          <ResolvedPermissionsTable
            auth={mockAuth}
            config={mockConfig}
            data={{ entries: {} }}
            resource={resource}
          />
        </MantineProvider>
      </QueryClientProvider>,
    );

    expect(container.querySelector("table")).toBeNull();
  });
});
