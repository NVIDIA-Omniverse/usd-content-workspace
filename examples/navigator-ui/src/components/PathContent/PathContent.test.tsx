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
import { waitFor } from "@testing-library/react";
import { BrowserRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { FolderContent } from "@/hooks/useFolderContentQuery";
import { PathMetadata } from "@/services/storage/metadataService";
import { Connection, useAppStore } from "@/state/appStore";
import { render, screen, userEvent } from "@/test-utils";
import { mockGetBoundingClientRect } from "@/test-utils/element";
import { createTestHttpError, testConnection } from "@/test-utils/msw";
import { createTestQueryClient } from "@/test-utils/utils";
import { PathContent } from "./PathContent";

const mocks = vi.hoisted(() => {
  return {
    useActiveConnection: vi.fn(),
    useConnection: vi.fn(),
    useFolderContentQuery: vi.fn(),
    useNotifications: vi.fn(),
    useClipboard: vi.fn(),
    useSetSelectedPaths: vi.fn(() => vi.fn()),
    useApi: vi.fn(),
    useFileDownload: vi.fn(),
    getPathMetadata: vi.fn(() => {
      return {
        acl: {
          value: ["read", "write"],
          etag: "",
        },
        tags: {
          value: {},
          etag: "",
        },
        thumbnails: [],
      } satisfies PathMetadata;
    }),
  };
});

vi.mock("@/state/appStore", async (importOriginal) => {
  const appStore = await importOriginal<typeof import("@/state/appStore")>();
  return {
    ...appStore,
    useActiveConnection: mocks.useActiveConnection,
    useConnection: mocks.useActiveConnection,
    useSetSelectedPaths: mocks.useSetSelectedPaths,
  };
});

vi.mock("@/hooks/useFolderContentQuery", async (importOriginal) => ({
  ...(await importOriginal()),
  useFolderContentQuery: mocks.useFolderContentQuery,
}));

vi.mock("@/hooks/useNotifications", () => ({
  useNotifications: mocks.useNotifications,
}));

vi.mock("@mantine/hooks", async (importOriginal) => {
  const hooks = await importOriginal<typeof import("@mantine/hooks")>();
  return {
    ...hooks,
    useClipboard: mocks.useClipboard,
  };
});

vi.mock("@/hooks/useFileDownload", () => ({
  useFileDownload: mocks.useFileDownload,
}));

vi.mock("@/hooks/usePathMetadata", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/hooks/usePathMetadata")>()),
  getPathMetadata: mocks.getPathMetadata,
}));

describe("PathContent", () => {
  const mockConnection: Partial<Connection> = {
    ...testConnection,
    currentFolder: "test-folder",
  };

  const mockShow = vi.fn();
  const mockStorageApi = {
    download: vi.fn(),
  };

  const component = () => (
    <BrowserRouter>
      <QueryClientProvider client={createTestQueryClient()}>
        <PathContent />
      </QueryClientProvider>
    </BrowserRouter>
  );

  beforeEach(() => {
    vi.clearAllMocks();

    mocks.useNotifications.mockReturnValue({
      show: mockShow,
    });

    mocks.useApi.mockReturnValue(mockStorageApi);
    mocks.useFileDownload.mockReturnValue({
      enqueue: vi.fn(),
      isInQueue: vi.fn(() => false),
    });

    mockGetBoundingClientRect((element) => element.getAttribute("aria-label") === "Folder content");
  });

  afterEach(() => {
    const initialState = useAppStore.getInitialState();
    useAppStore.setState(initialState, true);
  });

  it("shows message when there is no active connection", () => {
    mocks.useActiveConnection.mockReturnValue(null);
    render(component());
    expect(screen.getByText("No active connection.")).toBeVisible();
  });

  it("shows loading indicator when fetching data", () => {
    mocks.useActiveConnection.mockReturnValue(mockConnection);
    mocks.useFolderContentQuery.mockReturnValue({
      isFetching: true,
      isFetchingNextPage: false,
      data: { pages: [] },
    });
    render(component());
    expect(screen.getByRole("progressbar", { name: /Loading test-folder content/i })).toBeInTheDocument();
  });

  it("shows error message when fetching fails", () => {
    mocks.useActiveConnection.mockReturnValue(mockConnection);
    mocks.useFolderContentQuery.mockReturnValue({
      isError: true,
      isFetching: false,
      data: { pages: [] },
      error: "Test error.",
    });
    render(component());
    expect(screen.getByText("Failed to load test-folder content")).toBeVisible();
  });

  it("shows folder not found message on HTTP 404", () => {
    mocks.useActiveConnection.mockReturnValue(mockConnection);
    mocks.useFolderContentQuery.mockReturnValue({
      isError: true,
      isFetching: false,
      data: { pages: [] },
      error: createTestHttpError({ status: 404, statusText: "Not Found" }),
    });
    render(component());
    expect(screen.getByText("Folder not found")).toBeVisible();
    expect(screen.getByText("test-folder no longer exists or has been removed.")).toBeVisible();
  });

  it("displays a new folder modal", async () => {
    mocks.useActiveConnection.mockReturnValue(mockConnection);
    mocks.useFolderContentQuery.mockReturnValue({
      isLoading: false,
      data: {
        pages: [
          {
            files: new Map(),
            folders: new Map(),
          } as FolderContent,
        ],
      },
      fetchNextPage: vi.fn(),
      hasNextPage: false,
      isSuccess: true,
    });

    render(component());

    const folderContent = await screen.findByLabelText("test-folder content");
    await userEvent.pointer({ keys: "[MouseRight>]", target: folderContent });

    const newFolderOption = await screen.findByText("New folder");
    await userEvent.click(newFolderOption);

    const newFolderModal = await screen.findByRole("dialog", { name: "New Folder" });
    await waitFor(() => expect(newFolderModal).toBeVisible());
  });

  it("does not display folder context menu when viewing connection content", async () => {
    mocks.useActiveConnection.mockReturnValue({
      currentFolder: "",
      endpoint: "test-endpoint",
      info: {
        discovery: "http://discovery",
        auth: null,
      },
    });

    mocks.useFolderContentQuery.mockReturnValue({
      isLoading: false,
      data: {
        pages: [
          {
            isRoot: true,
            files: new Map(),
            folders: new Map(),
          } as FolderContent,
        ],
      },
    });

    render(component());

    const table = await screen.findByRole("table");
    await userEvent.pointer({ keys: "[MouseRight>]", target: table });

    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  });

  it("changes layout", async () => {
    mocks.useActiveConnection.mockReturnValue(mockConnection);
    mocks.useFolderContentQuery.mockReturnValue({
      isLoading: false,
      data: {
        pages: [
          {
            isRoot: true,
            files: new Map(),
            folders: new Map(),
          } as FolderContent,
        ],
      },
    });
    render(component());

    const changeLayout = await screen.findByRole("button", { name: "Change view layout to grid" });

    await userEvent.click(changeLayout);
    const grid = await screen.findByRole("grid");
    expect(grid).toBeVisible();

    await userEvent.click(changeLayout);
    const table = await screen.findByRole("table");
    expect(table).toBeVisible();
  });

  it("toggles detail panel", async () => {
    mocks.useActiveConnection.mockReturnValue(mockConnection);
    mocks.useFolderContentQuery.mockReturnValue({
      isLoading: false,
      data: {
        pages: [
          {
            isRoot: true,
            files: new Map(),
            folders: new Map(),
          } as FolderContent,
        ],
      },
    });
    render(component());

    const toggleDetails = await screen.findByRole("button", { name: "Hide details panel" });

    await userEvent.click(toggleDetails);
    expect(useAppStore.getState().detailsPanel).toBe(false);

    await userEvent.click(toggleDetails);
    expect(useAppStore.getState().detailsPanel).toBe(true);
  });
});
