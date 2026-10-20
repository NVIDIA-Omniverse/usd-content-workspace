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

import React from "react";
import { QueryClientProvider } from "@tanstack/react-query";
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { range } from "radash";
import { BrowserRouter } from "react-router-dom";
import { afterEach, beforeEach, vi } from "vitest";
import fileImage from "@/assets/file.png";
import folderImage from "@/assets/folder.png";
import serverImage from "@/assets/server.png";
import { PathContentGrid } from "@/components/PathContentGrid/PathContentGrid";
import { PathMetadata } from "@/services/storage/metadataService";
import { useAppStore } from "@/state/appStore";
import { Path } from "@/state/path";
import { render, userEvent } from "@/test-utils";
import { mockGetBoundingClientRect } from "@/test-utils/element";
import { createMockFile, createMockFolder, createMockTopLevelAddress } from "@/test-utils/mockPath";
import { testConnection } from "@/test-utils/msw";
import { createTestQueryClient } from "@/test-utils/utils";

const { useElementSize } = vi.hoisted(() => {
  return {
    useElementSize: vi.fn(() => ({
      ref: React.createRef(),
      width: 0,
      height: 0,
    })),
  };
});

vi.mock("@mantine/hooks", async (importOriginal) => {
  const hooks = await importOriginal<typeof import("@mantine/hooks")>();
  return {
    ...hooks,
    useElementSize,
  };
});

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

vi.mock("@/hooks/usePathMetadata", () => {
  const metadata: PathMetadata = {
    acl: {
      value: ["read", "write"],
      etag: "",
    },
    tags: {
      value: {},
      etag: "",
    },
    thumbnails: [],
  };

  return {
    getPathMetadata: vi.fn(() => metadata),
    usePathMetadata: vi.fn(() => ({ isSuccess: true, data: metadata })),
  };
});

describe("PathContentGrid", () => {
  beforeEach(() => {
    mockGetBoundingClientRect((element) => element.role === "grid", [600, 600]);
    useElementSize.mockImplementation(() => ({ ref: React.createRef(), width: 600, height: 600 }));

    useAppStore.setState({
      ...useAppStore.getInitialState(),
      connections: {
        [testConnection.endpoint]: testConnection,
      },
      endpoints: new Set([testConnection.endpoint]),
    });
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  const bucket1 = createMockTopLevelAddress("bucket1");
  const bucket2 = createMockTopLevelAddress("bucket2");
  const folder1 = createMockFolder("bucket1/folder1");
  const folder2 = createMockFolder("bucket1/folder2");
  const file1 = createMockFile("bucket1/file1.txt");
  const file2 = createMockFile("bucket1/file2.txt");

  const component = (data: Path[]) => (
    <BrowserRouter>
      <QueryClientProvider client={createTestQueryClient()}>
        <PathContentGrid connection={testConnection} data={data} />
      </QueryClientProvider>
    </BrowserRouter>
  );

  it("fits tiles in rows based on grid width", async () => {
    const data = range(1, 12, (index) => createMockFile(`${index}.txt`)).toArray();
    render(component(data));

    const rows = await screen.findAllByRole("row");
    for (const row of rows) {
      const cells = await within(row).findAllByRole("gridcell");
      expect(cells.length).toBe(3);
    }
  });

  it("fits tiles into a small grid", async () => {
    mockGetBoundingClientRect((element) => element.role === "grid", [100, 100]);
    useElementSize.mockImplementation(() => ({ ref: React.createRef(), width: 100, height: 100 }));

    const data = range(1, 10, (index) => createMockFile(`${index}.txt`)).toArray();
    render(component(data));

    const rows = await screen.findAllByRole("row");
    for (const row of rows) {
      const cells = await within(row).findAllByRole("gridcell");
      expect(cells.length).toBe(1);
    }
  });

  it("displays top level addresses", async () => {
    render(component([bucket1, bucket2]));

    const bucket1Tile = await screen.findByRole("gridcell", { name: bucket1.name });
    expect(bucket1Tile).toBeVisible();
    expect(within(bucket1Tile).getByRole("img")).toHaveAttribute("src", serverImage);

    const bucket2Tile = await screen.findByRole("gridcell", { name: bucket2.name });
    expect(bucket2Tile).toBeVisible();
    expect(within(bucket2Tile).getByRole("img")).toHaveAttribute("src", serverImage);
  });

  it("displays files and folders", async () => {
    render(component([folder1, folder2, file1, file2]));

    const folder1Tile = await screen.findByRole("gridcell", { name: folder1.name });
    expect(folder1Tile).toBeVisible();
    expect(within(folder1Tile).getByRole("img")).toHaveAttribute("src", folderImage);

    const folder2Tile = await screen.findByRole("gridcell", { name: folder2.name });
    expect(folder2Tile).toBeVisible();
    expect(within(folder2Tile).getByRole("img")).toHaveAttribute("src", folderImage);

    const file1Tile = await screen.findByRole("gridcell", { name: file1.name });
    expect(file1Tile).toBeVisible();
    expect(within(file1Tile).getByRole("img")).toHaveAttribute("src", fileImage);

    const file2Tile = await screen.findByRole("gridcell", { name: file2.name });
    expect(file2Tile).toBeVisible();
    expect(within(file2Tile).getByRole("img")).toHaveAttribute("src", fileImage);
  });

  it("displays context menu", async () => {
    render(component([file1]));

    const tile = await screen.findByRole("gridcell", { name: file1.name });
    await userEvent.pointer({ keys: "[MouseRight>]", target: tile });

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

  it("selects a file", async () => {
    render(component([file1]));

    const tile = await screen.findByRole("gridcell", { name: file1.name });
    expect(tile).toHaveAttribute("aria-selected", "false");
    await userEvent.click(tile);

    expect(tile).toHaveAttribute("aria-selected", "true");
  });

  it("scrolls the view to the selected path when details panel is toggled", async () => {
    mockGetBoundingClientRect(undefined, [100, 100]);
    useElementSize.mockImplementation(() => ({ ref: React.createRef(), width: 100, height: 100 }));

    const data = range(1, 50, (index) => createMockFile(`${index}.txt`)).toArray();
    render(component(data));

    const grid = await screen.findByRole("grid");
    fireEvent.scroll(grid, { target: { scrollTop: 4300 } });

    const tile = await screen.findByRole("gridcell", { name: "25.txt" });
    await userEvent.click(tile);

    // Simulate scroll change due to details panel visibility toggle
    fireEvent.scroll(grid, { target: { scrollTop: 0 } });

    const scrollTo = vi.fn();
    grid.scrollTo = scrollTo;

    act(() => {
      const state = useAppStore.getState();
      useAppStore.setState({
        detailsPanel: !state.detailsPanel,
      });
    });

    await waitFor(() => expect(scrollTo).toBeCalled());
  });

  it("navigates to a folder", async () => {
    render(component([folder1]));

    const tile = await screen.findByRole("gridcell", { name: folder1.name });
    await userEvent.dblClick(tile);

    expect(window.location.pathname).toBe("/http%3A%2F%2Ftest-endpoint/path/bucket1%2Ffolder1%2F");
  });

  it("selects multiple tiles", async () => {
    render(component([folder1, folder2, file1, file2]));

    const folder1Checkbox = await screen.findByRole("checkbox", { name: `Select ${folder1.name}` });
    await userEvent.click(folder1Checkbox);

    const folder1Tile = await screen.findByRole("gridcell", { name: folder1.name });
    expect(folder1Tile).toHaveAttribute("aria-selected", "true");

    const folder2Checkbox = await screen.findByRole("checkbox", { name: `Select ${folder2.name}` });
    await userEvent.click(folder2Checkbox);

    const folder2Tile = await screen.findByRole("gridcell", { name: folder2.name });
    expect(folder2Tile).toHaveAttribute("aria-selected", "true");
    expect(folder1Tile).toHaveAttribute("aria-selected", "true");

    const file1Checkbox = await screen.findByRole("checkbox", { name: `Select ${file1.name}` });
    await userEvent.click(file1Checkbox);

    const file1Tile = await screen.findByRole("gridcell", { name: file1.name });
    expect(file1Tile).toHaveAttribute("aria-selected", "true");
    expect(folder2Tile).toHaveAttribute("aria-selected", "true");
    expect(folder1Tile).toHaveAttribute("aria-selected", "true");

    await userEvent.click(folder2Checkbox);
    expect(file1Tile).toHaveAttribute("aria-selected", "true");
    expect(folder2Tile).toHaveAttribute("aria-selected", "false");
    expect(folder1Tile).toHaveAttribute("aria-selected", "true");
  });

  it("clears tile selection when clicking an empty grid area", async () => {
    render(component([folder1, folder2, file1, file2]));

    const folder1Checkbox = await screen.findByRole("checkbox", { name: `Select ${folder1.name}` });
    await userEvent.click(folder1Checkbox);

    const folder2Checkbox = await screen.findByRole("checkbox", { name: `Select ${folder2.name}` });
    await userEvent.click(folder2Checkbox);

    const folder1Tile = await screen.findByRole("gridcell", { name: folder1.name });
    const folder2Tile = await screen.findByRole("gridcell", { name: folder2.name });
    expect(folder1Tile).toHaveAttribute("aria-selected", "true");
    expect(folder2Tile).toHaveAttribute("aria-selected", "true");

    await userEvent.click(screen.getByRole("grid"));

    await waitFor(() => expect(folder1Tile).toHaveAttribute("aria-selected", "false"));
    await waitFor(() => expect(folder2Tile).toHaveAttribute("aria-selected", "false"));
    expect(useAppStore.getState().connections[testConnection.endpoint]?.selectedPaths).toEqual(new Set());
  });

  it("shows context menu for multiple files", async () => {
    render(component([file1, file2]));

    const file1Checkbox = await screen.findByRole("checkbox", { name: `Select ${file1.name}` });
    await userEvent.click(file1Checkbox);

    const file2Checkbox = await screen.findByRole("checkbox", { name: `Select ${file2.name}` });
    await userEvent.click(file2Checkbox);

    const file1Tile = await screen.findByRole("gridcell", { name: file1.name });
    await userEvent.pointer({ keys: "[MouseRight>]", target: file1Tile });

    const downloadMenuItem = await screen.findByText("Download");
    expect(downloadMenuItem).toBeVisible();

    // Rename option is not available for multi-selection
    const renameMenuItem = screen.queryByText("Rename");
    expect(renameMenuItem).toBeNull();
  });
});
