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
import { act, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { BrowserRouter } from "react-router-dom";
import { afterEach, beforeEach, expect, vi } from "vitest";
import { PathMetadata } from "@/services/storage/metadataService";
import { useAppStore } from "@/state/appStore";
import { FileListItem, FolderListItem, PathType } from "@/state/path";
import { render, screen, within } from "@/test-utils";
import { mockGetBoundingClientRect } from "@/test-utils/element";
import { testConnection } from "@/test-utils/msw";
import { createTestQueryClient } from "@/test-utils/utils";
import { PathContentTable } from "./PathContentTable";

vi.mock("@/hooks/usePathClipboard", () => ({
  usePathClipboard: vi.fn(() => ({
    empty: vi.fn(() => true),
    items: [],
    operation: "Copy",
    sourcePath: "",
    clear: vi.fn(),
    copy: vi.fn(),
    cut: vi.fn(),
    paste: {
      mutate: vi.fn(),
      mutateAsync: vi.fn(),
      isPending: false,
      isError: false,
      error: null,
    },
  })),
}));

vi.mock("@/hooks/usePathMetadata", () => ({
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
}));

describe("PathContentTable", () => {
  const file1: FileListItem = {
    name: "file1.txt",
    type: PathType.File,
    displayedType: "TXT",
    isFolderLike: false,
    bytes: 1024,
    size: "1.00 KB",
    modified: "02/27/2024 12:00:00",
    resourceIdentity: "test-folder/file1.txt",
    resourceAddress: "test-folder/file1.txt",
    metadata: {
      dataObjectSize: 1024,
      lastModifiedTimestamp: "2024-03-20T10:00:00Z",
    },
  };

  const file2: FileListItem = {
    name: "file2.txt",
    type: PathType.File,
    displayedType: "TXT",
    isFolderLike: false,
    bytes: 2048,
    size: "2.00 KB",
    modified: "02/27/2024 12:00:00",
    resourceIdentity: "test-folder/file2.txt",
    resourceAddress: "test-folder/file2.txt",
    metadata: {
      dataObjectSize: 2048,
      lastModifiedTimestamp: "2024-03-20T11:00:00Z",
    },
  };

  const subfolder1: FolderListItem = {
    name: "subfolder1",
    type: PathType.Folder,
    displayedType: "Folder",
    isFolderLike: true,
    resourceAddress: "test-folder/subfolder1",
  };

  const subfolder2: FolderListItem = {
    name: "subfolder2",
    type: PathType.Folder,
    displayedType: "Folder",
    isFolderLike: true,
    resourceAddress: "test-folder/subfolder2",
  };

  const component = () => (
    <BrowserRouter>
      <QueryClientProvider client={createTestQueryClient()}>
        <PathContentTable connection={testConnection} data={[subfolder1, subfolder2, file1, file2]} />
      </QueryClientProvider>
    </BrowserRouter>
  );

  beforeEach(() => {
    mockGetBoundingClientRect();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("displays folders and files", () => {
    render(component());

    expect(screen.getByRole("row", { name: "subfolder1" })).toBeVisible();
    expect(screen.getByRole("row", { name: "subfolder2" })).toBeVisible();

    const file1 = screen.getByRole("row", { name: "file1.txt" });
    expect(file1).toBeVisible();
    expect(within(file1).getByText("1.00 KB")).toBeVisible();
    expect(within(file1).getByText("TXT")).toBeVisible();

    const file2 = screen.getByRole("row", { name: "file2.txt" });
    expect(file2).toBeVisible();
    expect(within(file2).getByText("2.00 KB")).toBeVisible();
    expect(within(file2).getByText("TXT")).toBeVisible();
  });

  it("navigates to a folder after a double-click", async () => {
    render(component());

    const folder = screen.getByRole("row", { name: "subfolder1" });
    await userEvent.dblClick(folder);

    expect(window.location.pathname).toBe("/http%3A%2F%2Ftest-endpoint/path/test-folder%2Fsubfolder1%2F");
  });

  it("displays context menu", async () => {
    render(component());

    const file = screen.getByRole("row", { name: "file1.txt" });
    await userEvent.pointer({ keys: "[MouseRight>]", target: file });

    const downloadMenuItem = await screen.findByText("Download");
    expect(downloadMenuItem).toBeVisible();

    const copyMenuItem = await screen.findByText("Copy");
    expect(copyMenuItem).toBeVisible();

    const renameMenuItem = await screen.findByText("Rename");
    expect(renameMenuItem).toBeVisible();

    const cutMenuItem = await screen.findByText("Cut");
    expect(cutMenuItem).toBeVisible();

    const deleteMenuItem = await screen.findByText("Delete");
    expect(deleteMenuItem).toBeVisible();
  });

  it("displays a delete modal", async () => {
    render(component());

    const file = screen.getByRole("row", { name: "file1.txt" });
    await userEvent.pointer({ keys: "[MouseRight>]", target: file });

    const deleteOption = await screen.findByText("Delete");
    await userEvent.click(deleteOption);

    const deleteModal = await screen.findByRole("dialog", { name: "Delete" });
    await waitFor(() => expect(deleteModal).toBeVisible());

    const deleteText = await screen.findByText(`Are you sure you want to delete ${file1.name}?`);
    expect(deleteText).toBeVisible();
  });

  it("displays a delete modal for multiple paths", async () => {
    render(component());

    const file1Checkbox = await screen.findByTitle(`Select ${file1.name}`);
    await userEvent.click(file1Checkbox);

    const file2Checkbox = await screen.findByTitle(`Select ${file2.name}`);
    await userEvent.click(file2Checkbox);

    const file2Row = await screen.findByText(file2.name);
    await userEvent.pointer({ keys: "[MouseRight>]", target: file2Row });

    const deleteOption = await screen.findByText("Delete");
    await userEvent.click(deleteOption);

    const deleteModal = await screen.findByRole("dialog", { name: "Delete" });
    await waitFor(() => expect(deleteModal).toBeVisible());

    const deleteText = await screen.findByText(`Are you sure you want to delete 2 items?`);
    expect(deleteText).toBeVisible();
  });

  it("initializes row selection from selectedPaths when component mounts", async () => {
    const addConnection = useAppStore.getState().addConnection;
    act(() => {
      addConnection(testConnection.endpoint, testConnection.info);
    });

    const setSelectedPaths = useAppStore.getState().setSelectedPaths;
    act(() => {
      setSelectedPaths(testConnection.endpoint, [file1, subfolder1]);
    });

    render(component());

    const file1Checkbox = screen.getByTitle(`Select ${file1.name}`);
    const subfolder1Checkbox = screen.getByTitle(`Select ${subfolder1.name}`);
    await waitFor(() => expect(file1Checkbox).toBeChecked());
    await waitFor(() => expect(subfolder1Checkbox).toBeChecked());

    const file2Checkbox = screen.getByTitle(`Select ${file2.name}`);
    const subfolder2Checkbox = screen.getByTitle(`Select ${subfolder2.name}`);

    expect(file2Checkbox).not.toBeChecked();
    expect(subfolder2Checkbox).not.toBeChecked();
  });

  it("updates row selection when selectedPaths changes", async () => {
    const addConnection = useAppStore.getState().addConnection;
    act(() => {
      addConnection(testConnection.endpoint, testConnection.info);
    });

    const { rerender } = render(component());

    const file1Checkbox = screen.getByTitle(`Select ${file1.name}`);
    expect(file1Checkbox).not.toBeChecked();

    const setSelectedPaths = useAppStore.getState().setSelectedPaths;
    act(() => {
      setSelectedPaths(testConnection.endpoint, [file1]);
    });

    rerender(component());

    const updatedFile1Checkbox = screen.getByTitle(`Select ${file1.name}`);
    await waitFor(() => expect(updatedFile1Checkbox).toBeChecked());
  });

  it("handles selectedPaths that don't exist in current data", async () => {
    const addConnection = useAppStore.getState().addConnection;
    act(() => {
      addConnection(testConnection.endpoint, testConnection.info);
    });

    const nonExistentPath: FileListItem = {
      name: "non-existent.txt",
      type: PathType.File,
      displayedType: "TXT",
      isFolderLike: false,
      bytes: 512,
      size: "512 B",
      modified: "02/27/2024 12:00:00",
      resourceIdentity: "test-folder/non-existent.txt",
      resourceAddress: "test-folder/non-existent.txt",
      metadata: {
        dataObjectSize: 512,
        lastModifiedTimestamp: "2024-03-20T12:00:00Z",
      },
    };

    const setSelectedPaths = useAppStore.getState().setSelectedPaths;
    act(() => {
      setSelectedPaths(testConnection.endpoint, [file1, nonExistentPath]);
    });

    render(component());

    const file1Checkbox = screen.getByTitle(`Select ${file1.name}`);
    await waitFor(() => expect(file1Checkbox).toBeChecked());

    const file2Checkbox = screen.getByTitle(`Select ${file2.name}`);
    expect(file2Checkbox).not.toBeChecked();
  });

  it("clears row selection when selectedPaths is empty", async () => {
    // First, ensure the connection exists in the store
    const addConnection = useAppStore.getState().addConnection;
    act(() => {
      addConnection(testConnection.endpoint, testConnection.info);
    });

    const setSelectedPaths = useAppStore.getState().setSelectedPaths;
    act(() => {
      setSelectedPaths(testConnection.endpoint, [file1, file2]);
    });

    const { rerender } = render(component());

    const file1Checkbox = screen.getByTitle(`Select ${file1.name}`);
    const file2Checkbox = screen.getByTitle(`Select ${file2.name}`);
    await waitFor(() => expect(file1Checkbox).toBeChecked());
    await waitFor(() => expect(file2Checkbox).toBeChecked());

    act(() => {
      setSelectedPaths(testConnection.endpoint, []);
    });

    rerender(component());

    const updatedFile1Checkbox = screen.getByTitle(`Select ${file1.name}`);
    const updatedFile2Checkbox = screen.getByTitle(`Select ${file2.name}`);

    await waitFor(() => expect(updatedFile1Checkbox).not.toBeChecked());
    await waitFor(() => expect(updatedFile2Checkbox).not.toBeChecked());
  });

  it("clears row selection when clicking an empty content area", async () => {
    const addConnection = useAppStore.getState().addConnection;
    act(() => {
      addConnection(testConnection.endpoint, testConnection.info);
    });

    render(component());

    const file1Checkbox = screen.getByTitle(`Select ${file1.name}`);
    const file2Checkbox = screen.getByTitle(`Select ${file2.name}`);
    await userEvent.click(file1Checkbox);
    await userEvent.click(file2Checkbox);

    await waitFor(() => expect(file1Checkbox).toBeChecked());
    await waitFor(() => expect(file2Checkbox).toBeChecked());

    await userEvent.click(screen.getByLabelText("Folder content"));

    await waitFor(() => expect(file1Checkbox).not.toBeChecked());
    await waitFor(() => expect(file2Checkbox).not.toBeChecked());
    expect(useAppStore.getState().connections[testConnection.endpoint]?.selectedPaths).toEqual(new Set());
  });
});
