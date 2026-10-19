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

import { QueryClientProvider } from "@tanstack/react-query";
import { http, HttpResponse } from "msw";
import { setupServer } from "msw/node";
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { useAppStore } from "@/state/appStore";
import { rootFolderPath } from "@/state/slices/connectionSlice";
import { render, screen, userEvent, waitFor } from "@/test-utils";
import { testConnection, testEndpoints } from "@/test-utils/msw";
import { createTestQueryClient } from "@/test-utils/utils";
import { PathCurrentFolderDetails } from "./PathCurrentFolderDetails";

const handlers = [
  http.post("http://*/v1alpha/metadata/:resourceAddress", ({ params }) => {
    const resourceAddress = decodeURIComponent(params.resourceAddress as string);

    if (resourceAddress === "http://test-endpoint/folder1/") {
      return HttpResponse.json({
        acl: {
          etag: "etag-123",
          value: ["write", "read"],
        },
      });
    }

    if (resourceAddress === "http://test-endpoint/folder2/") {
      return HttpResponse.json({
        acl: {
          etag: "etag-456",
          value: ["read"],
        },
      });
    }

    if (resourceAddress === "http://test-endpoint") {
      return HttpResponse.json({
        acl: {
          etag: "etag-789",
          value: ["write"],
        },
      });
    }

    return HttpResponse.json({ error: "Not found" }, { status: 404 });
  }),
  ...testEndpoints,
];

const server = setupServer(...handlers);

beforeAll(() => {
  server.listen();
});

beforeEach(() => {
  useAppStore.setState({ detailsPanelTab: "details" });
});

afterEach(() => {
  server.resetHandlers();
});

afterAll(() => {
  server.close();
});

describe("PathCurrentFolderDetails", () => {
  const renderComponent = (connection = testConnection) => {
    return render(
      <QueryClientProvider client={createTestQueryClient()}>
        <PathCurrentFolderDetails connection={connection} />
      </QueryClientProvider>
    );
  };

  it("renders folder details with read and write permissions", async () => {
    const connection = {
      ...testConnection,
      currentFolder: "http://test-endpoint/folder1/",
    };

    renderComponent(connection);

    expect(screen.getByText("folder1")).toBeInTheDocument();
    expect(screen.getByText("http://test-endpoint/folder1/")).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.getByText("Resolved Access")).toBeInTheDocument();
      expect(screen.getByText("read, write")).toBeInTheDocument();
    });
  });

  it("does not render details when a connection node is selected", () => {
    const connectionNode = {
      ...testConnection,
      currentFolder: rootFolderPath, // the connection node itself is selected
    };

    renderComponent(connectionNode);

    // Should render the connection endpoint name and address
    expect(screen.getByText("test-endpoint")).toBeInTheDocument();
    expect(screen.getByText("http://test-endpoint")).toBeInTheDocument();

    // But should NOT render the Details tab or any other tabs
    expect(screen.queryByRole("tab", { name: "Details" })).not.toBeInTheDocument();
    expect(screen.queryByRole("tab", { name: "Permissions" })).not.toBeInTheDocument();
    expect(screen.queryByRole("tab", { name: "Tags" })).not.toBeInTheDocument();
  });

  describe("Remembered tab", () => {
    const folder1 = { ...testConnection, currentFolder: "http://test-endpoint/folder1/" };
    const folder2 = { ...testConnection, currentFolder: "http://test-endpoint/folder2/" };

    it("selects the tab remembered in the store", () => {
      useAppStore.setState({ detailsPanelTab: "tags" });

      renderComponent(folder1);

      expect(screen.getByRole("tab", { name: "Tags" })).toHaveAttribute("aria-selected", "true");
    });

    it("stores the selected tab when the user switches tabs", async () => {
      renderComponent(folder1);

      await userEvent.click(screen.getByRole("tab", { name: "Permissions" }));

      expect(useAppStore.getState().detailsPanelTab).toBe("permissions");
    });

    it("keeps the selected tab when switching to another folder", async () => {
      const { unmount } = renderComponent(folder1);

      await userEvent.click(screen.getByRole("tab", { name: "Permissions" }));
      unmount();

      renderComponent(folder2);

      expect(screen.getByRole("tab", { name: "Permissions" })).toHaveAttribute("aria-selected", "true");
    });
  });
});
