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
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { useAppStore } from "@/state/appStore";
import { render, screen, userEvent, waitFor } from "@/test-utils";
import { createMockFile, createMockFolder, createMockTopLevelAddress } from "@/test-utils/mockPath";
import { testConnection, testEndpoints } from "@/test-utils/msw";
import { createTestQueryClient } from "@/test-utils/utils";
import { PathDetails } from "./PathDetails";

// Mock the FileVersions component since it's tested separately
vi.mock("@/components/FileVersions", () => ({
  FileVersions: vi.fn(() => <div data-testid="file-versions">File Versions Component</div>),
}));

const handlers = [
  http.post("http://*/v1alpha/metadata/:resourceAddress", ({ params }) => {
    const resourceAddress = decodeURIComponent(params.resourceAddress as string);

    if (resourceAddress === "http://test-server/bucket1/test-file.txt") {
      return HttpResponse.json({
        acl: {
          etag: "etag-123",
          value: ["read", "write"],
        },
        tags: { etag: "", value: {} },
      });
    }

    if (resourceAddress === "http://test-server/bucket1/test-folder/") {
      return HttpResponse.json({
        acl: {
          etag: "etag-456",
          value: ["read"],
        },
        tags: { etag: "", value: {} },
      });
    }

    if (resourceAddress === "http://test-server/bucket1/") {
      return HttpResponse.json({
        acl: {
          etag: "etag-789",
          value: ["read", "write"],
        },
        tags: { etag: "", value: {} },
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
  vi.clearAllMocks();
});

afterAll(() => {
  server.close();
});

describe("PathDetails", () => {
  const renderComponent = (path: any) => {
    return render(
      <QueryClientProvider client={createTestQueryClient()}>
        <PathDetails path={path} connection={testConnection} />
      </QueryClientProvider>
    );
  };

  describe("File path", () => {
    it("renders file details correctly", async () => {
      const filePath = createMockFile("http://test-server/bucket1/test-file.txt");

      renderComponent(filePath);

      expect(screen.getByText("test-file.txt")).toBeInTheDocument();
      expect(screen.getByText("http://test-server/bucket1/test-file.txt")).toBeInTheDocument();

      expect(screen.getByText("Type")).toBeInTheDocument();
      expect(screen.getByText("TXT")).toBeInTheDocument();
      expect(screen.getByText("Size")).toBeInTheDocument();
      expect(screen.getByText("1.46 KiB")).toBeInTheDocument();
      expect(screen.getByText("Modified")).toBeInTheDocument();
      expect(screen.getByText("02/27/2024 12:00:00")).toBeInTheDocument();

      await waitFor(() => {
        expect(screen.getByText("Resolved Access")).toBeInTheDocument();
        expect(screen.getByText("read, write")).toBeInTheDocument();
      });
    });
  });

  describe("Folder path", () => {
    it("renders folder details correctly", async () => {
      const folderPath = createMockFolder("http://test-server/bucket1/test-folder/");

      renderComponent(folderPath);

      expect(screen.getByText("test-folder")).toBeInTheDocument();
      expect(screen.getByText("http://test-server/bucket1/test-folder/")).toBeInTheDocument();

      expect(screen.getByText("Type")).toBeInTheDocument();
      expect(screen.getByText("Folder")).toBeInTheDocument();
      expect(screen.queryByText("Size")).not.toBeInTheDocument();
      expect(screen.queryByText("Modified")).not.toBeInTheDocument();

      await waitFor(() => {
        expect(screen.getByText("Resolved Access")).toBeInTheDocument();
        expect(screen.getByText("read")).toBeInTheDocument();
      });
    });
  });

  describe("Top-level address path", () => {
    it("renders top-level address details correctly", async () => {
      const topLevelPath = createMockTopLevelAddress("http://test-server/bucket1/");

      renderComponent(topLevelPath);

      expect(screen.getByText("bucket1")).toBeInTheDocument();
      expect(screen.getByText("http://test-server/bucket1/")).toBeInTheDocument();

      expect(screen.getByText("Type")).toBeInTheDocument();
      expect(screen.getByText("Bucket")).toBeInTheDocument();
      expect(screen.queryByText("Size")).not.toBeInTheDocument();
      expect(screen.queryByText("Modified")).not.toBeInTheDocument();

      await waitFor(() => {
        expect(screen.getByText("Resolved Access")).toBeInTheDocument();
        expect(screen.getByText("read, write")).toBeInTheDocument();
      });
    });
  });

  describe("Remembered tab", () => {
    it("selects the tab remembered in the store", () => {
      useAppStore.setState({ detailsPanelTab: "permissions" });

      renderComponent(createMockFile("http://test-server/bucket1/test-file.txt"));

      expect(screen.getByRole("tab", { name: "Permissions" })).toHaveAttribute("aria-selected", "true");
    });

    it("stores the selected tab when the user switches tabs", async () => {
      renderComponent(createMockFile("http://test-server/bucket1/test-file.txt"));

      await userEvent.click(screen.getByRole("tab", { name: "Tags" }));

      expect(useAppStore.getState().detailsPanelTab).toBe("tags");
    });

    it("keeps the selected tab when switching to another path", async () => {
      const { unmount } = renderComponent(createMockFile("http://test-server/bucket1/test-file.txt"));

      await userEvent.click(screen.getByRole("tab", { name: "Permissions" }));
      unmount();

      renderComponent(createMockFolder("http://test-server/bucket1/test-folder/"));

      expect(screen.getByRole("tab", { name: "Permissions" })).toHaveAttribute("aria-selected", "true");
    });
  });
});
