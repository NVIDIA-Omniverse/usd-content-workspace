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
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ContextMenuProvider } from "mantine-contextmenu";
import { http, HttpResponse } from "msw";
import { setupServer } from "msw/node";
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { MantineProvider } from "@mantine/core";
import { ModalsProvider } from "@/providers/ModalsProvider/ModalsProvider";
import { createFileListItem, createFolderListItem, createTopLevelAddressListItem, Path } from "@/state/path";
import { Connection } from "@/state/slices/connectionSlice";
import { testConnection } from "@/test-utils/msw";
import { createTestQueryClient } from "@/test-utils/utils";
import { usePathContextMenu } from "./usePathContextMenu";

const mockDownload = vi.fn();
const mockUseFileDownload = vi.fn<(connection: Connection | undefined) => void>();
const mockArchiveDownload = vi.fn();
const mockUseArchiveDownload = vi.fn<(connection: Connection | undefined) => void>();
let mockArchiveAvailable = false;
const mockDeleteMutate = vi.fn();
const mockUseDeletePathMutation = vi.fn<(connection: Connection | undefined) => void>();
const mockClipboardCopy = vi.fn();
const mockClipboardCut = vi.fn();
const mockClipboardPaste = vi.fn();
const mockShowDirectoryPicker = vi.fn();

vi.mock("@/hooks/useFileDownload", () => ({
  useFileDownload: (connection: Connection | undefined) => {
    mockUseFileDownload(connection);
    return {
      download: mockDownload,
    };
  },
}));

vi.mock("@/hooks/useArchiveDownload", () => ({
  useArchiveDownload: (connection: Connection | undefined) => {
    mockUseArchiveDownload(connection);
    return {
      available: mockArchiveAvailable,
      download: mockArchiveDownload,
    };
  },
}));

vi.mock("@/hooks/useDeletePathMutation", () => ({
  useDeletePathMutation: (connection: Connection | undefined) => {
    mockUseDeletePathMutation(connection);
    return {
      mutate: mockDeleteMutate,
    };
  },
}));

vi.mock("@/hooks/usePathClipboard", () => ({
  usePathClipboard: () => ({
    copy: mockClipboardCopy,
    cut: mockClipboardCut,
    paste: {
      mutate: mockClipboardPaste,
    } as any,
  }),
}));

vi.stubGlobal("showDirectoryPicker", mockShowDirectoryPicker);

const server = setupServer(
  http.get("http://*/api/v2/services", () => new HttpResponse(null, { status: 404 })),
  http.get("http://*/api/v1/services", () =>
    HttpResponse.json({
      services: [{ id: "storage", name: "Storage Service", type: "storage", rest: "http://test-endpoint" }],
    })
  ),
  http.post("http://*/v1alpha/metadata/*", () => {
    return HttpResponse.json({
      acl: {
        etag: "test-etag",
        value: ["read", "write"],
      },
    });
  })
);

beforeAll(() => server.listen());
afterEach(() => {
  server.resetHandlers();
  vi.clearAllMocks();
  mockArchiveAvailable = false;
});
afterAll(() => server.close());

const TestComponent = ({ connection, paths }: { connection: Connection; paths: Path[] }) => {
  const contextMenuHandler = usePathContextMenu(connection);

  return (
    <div
      onContextMenu={(e) => {
        void contextMenuHandler(paths)(e);
      }}
      style={{ width: 200, height: 200, backgroundColor: "lightgray" }}
    >
      Test Area
    </div>
  );
};

const component = (connection: Connection, paths: Path[]) => {
  return (
    <QueryClientProvider client={createTestQueryClient()}>
      <MantineProvider>
        <ContextMenuProvider>
          <ModalsProvider>
            <TestComponent connection={connection} paths={paths} />
          </ModalsProvider>
        </ContextMenuProvider>
      </MantineProvider>
    </QueryClientProvider>
  );
};

describe("usePathContextMenu", () => {
  const mockConnection: Connection = testConnection;

  const testFile = createFileListItem({
    resourceAddress: "/test-file.txt",
    resourceIdentity: "test-file-id",
    metadata: {
      dataObjectSize: 1024,
      lastModifiedTimestamp: "2023-01-01T00:00:00.000Z",
    },
  });

  const testFolder = createFolderListItem("/test-folder");
  const topLevelAddress = createTopLevelAddressListItem("/top-level");

  it("should not show context menu for empty paths", async () => {
    render(component(mockConnection, []));

    const testArea = screen.getByText("Test Area");
    await userEvent.pointer({ keys: "[MouseRight]", target: testArea });

    expect(screen.queryByText("Download")).not.toBeInTheDocument();
  });

  it("should show context menu options for single file", async () => {
    render(component(mockConnection, [testFile]));

    const testArea = screen.getByText("Test Area");
    await userEvent.pointer({ keys: "[MouseRight]", target: testArea });

    expect(await screen.findByText("Download")).toBeVisible();
    expect(await screen.findByText("Copy")).toBeVisible();
    expect(await screen.findByText("Rename")).toBeVisible();
    expect(await screen.findByText("Cut")).toBeVisible();
    expect(await screen.findByText("Delete")).toBeVisible();
  });

  it("should show context menu options for multiple paths", async () => {
    render(component(mockConnection, [testFile, testFolder]));

    const testArea = screen.getByText("Test Area");
    await userEvent.pointer({ keys: "[MouseRight]", target: testArea });

    expect(await screen.findByText("Download")).toBeVisible();
    expect(await screen.findByText("Copy")).toBeVisible();
    expect(await screen.findByText("Cut")).toBeVisible();
    expect(await screen.findByText("Delete")).toBeVisible();
    expect(screen.queryByText("Rename")).not.toBeInTheDocument();
  });

  it("should hide rename, cut, and delete options for top level addresses", async () => {
    render(component(mockConnection, [topLevelAddress]));

    const testArea = screen.getByText("Test Area");
    await userEvent.pointer({ keys: "[MouseRight]", target: testArea });

    expect(await screen.findByText("Download")).toBeVisible();
    expect(await screen.findByText("Copy")).toBeVisible();
    expect(screen.queryByText("Rename")).not.toBeInTheDocument();
    expect(screen.queryByText("Cut")).not.toBeInTheDocument();
    expect(screen.queryByText("Delete")).not.toBeInTheDocument();
  });

  it("should hide cut and delete options for multiple top level addresses", async () => {
    const topLevelAddress2 = createTopLevelAddressListItem("/top-level-2");

    render(component(mockConnection, [topLevelAddress, topLevelAddress2]));

    const testArea = screen.getByText("Test Area");
    await userEvent.pointer({ keys: "[MouseRight]", target: testArea });

    expect(await screen.findByText("Download")).toBeVisible();
    expect(await screen.findByText("Copy")).toBeVisible();
    expect(screen.queryByText("Cut")).not.toBeInTheDocument();
    expect(screen.queryByText("Delete")).not.toBeInTheDocument();
  });

  it("should disable read operations for paths without read permission", async () => {
    server.use(
      http.post("http://*/v1alpha/metadata/*", () => {
        return HttpResponse.json({
          acl: {
            etag: "test-etag",
            value: ["write"],
          },
          tags: {
            etag: "",
            value: {},
          },
        });
      })
    );

    render(component(mockConnection, [testFile]));

    const testArea = screen.getByText("Test Area");
    await userEvent.pointer({ keys: "[MouseRight]", target: testArea });

    const downloadButton = await screen.findByText("Download");
    expect(downloadButton).toBeVisible();
    expect(downloadButton.closest("button")).toBeDisabled();

    const copyButton = await screen.findByText("Copy");
    expect(copyButton).toBeVisible();
    expect(copyButton.closest("button")).toBeDisabled();
  });

  it("should disable write operations for paths without write permission", async () => {
    server.use(
      http.post("http://*/v1alpha/metadata/*", () => {
        return HttpResponse.json({
          acl: {
            etag: "test-etag",
            value: ["read"],
          },
          tags: { etag: "", value: {} },
        });
      })
    );

    render(component(mockConnection, [testFile]));

    const testArea = screen.getByText("Test Area");
    await userEvent.pointer({ keys: "[MouseRight]", target: testArea });

    const renameButton = await screen.findByText("Rename");
    expect(renameButton).toBeVisible();
    expect(renameButton.closest("button")).toBeDisabled();

    const cutButton = await screen.findByText("Cut");
    expect(cutButton).toBeVisible();
    expect(cutButton.closest("button")).toBeDisabled();

    const deleteButton = await screen.findByText("Delete");
    expect(deleteButton).toBeVisible();
    expect(deleteButton.closest("button")).toBeDisabled();
  });

  it("should disable operations for multiple paths when any path lacks required permission", async () => {
    let callCount = 0;
    server.use(
      http.post("http://*/v1alpha/metadata/*", () => {
        callCount++;
        return HttpResponse.json({
          acl: {
            etag: "test-etag",
            value: callCount === 1 ? ["read", "write"] : ["read"],
          },
          tags: { etag: "", value: {} },
        });
      })
    );

    render(component(mockConnection, [testFile, testFolder]));

    const testArea = screen.getByText("Test Area");
    await userEvent.pointer({ keys: "[MouseRight]", target: testArea });

    const cutButton = await screen.findByText("Cut");
    expect(cutButton).toBeVisible();
    expect(cutButton.closest("button")).toBeDisabled();

    const deleteButton = await screen.findByText("Delete");
    expect(deleteButton).toBeVisible();
    expect(deleteButton.closest("button")).toBeDisabled();
  });

  it("should handle download operation click", async () => {
    const mockDirectoryHandle = {};
    mockShowDirectoryPicker.mockResolvedValue(mockDirectoryHandle);

    render(component(mockConnection, [testFile]));

    const testArea = screen.getByText("Test Area");
    await userEvent.pointer({ keys: "[MouseRight]", target: testArea });

    const downloadButton = await screen.findByText("Download");
    expect(downloadButton).toBeVisible();

    await userEvent.click(downloadButton);

    expect(mockShowDirectoryPicker).toHaveBeenCalledWith({ mode: "readwrite" });
    expect(mockDownload).toHaveBeenCalledWith([testFile], mockDirectoryHandle);
  });

  it("should use the archive download when the archive service is available", async () => {
    mockArchiveAvailable = true;

    render(component(mockConnection, [testFile]));

    const testArea = screen.getByText("Test Area");
    await userEvent.pointer({ keys: "[MouseRight]", target: testArea });

    const downloadButton = await screen.findByText("Download");
    await userEvent.click(downloadButton);

    expect(mockArchiveDownload).toHaveBeenCalledWith([testFile]);
    expect(mockShowDirectoryPicker).not.toHaveBeenCalled();
  });

  it("should resolve download, archive, and delete actions for the menu's connection", () => {
    const otherConnection: Connection = { ...testConnection, endpoint: "http://other-endpoint" };

    render(component(otherConnection, [testFile]));

    expect(mockUseFileDownload).toHaveBeenCalledWith(otherConnection);
    expect(mockUseArchiveDownload).toHaveBeenCalledWith(otherConnection);
    expect(mockUseDeletePathMutation).toHaveBeenCalledWith(otherConnection);
  });

  it("should handle copy operation click", async () => {
    render(component(mockConnection, [testFile]));

    const testArea = screen.getByText("Test Area");
    await userEvent.pointer({ keys: "[MouseRight]", target: testArea });

    const copyButton = await screen.findByText("Copy");
    expect(copyButton).toBeVisible();

    await userEvent.click(copyButton);

    expect(mockClipboardCopy).toHaveBeenCalledWith([testFile]);
  });

  it("should handle cut operation click", async () => {
    render(component(mockConnection, [testFile]));

    const testArea = screen.getByText("Test Area");
    await userEvent.pointer({ keys: "[MouseRight]", target: testArea });

    const cutButton = await screen.findByText("Cut");
    expect(cutButton).toBeVisible();

    await userEvent.click(cutButton);

    expect(mockClipboardCut).toHaveBeenCalledWith([testFile]);
  });

  it("should open rename modal when rename option is clicked", async () => {
    render(component(mockConnection, [testFile]));

    const testArea = screen.getByText("Test Area");
    await userEvent.pointer({ keys: "[MouseRight]", target: testArea });

    const renameButton = await screen.findByText("Rename");
    expect(renameButton).toBeVisible();

    await userEvent.click(renameButton);

    expect(await screen.findByRole("dialog")).toBeInTheDocument();
  });

  it("should open delete modal when delete option is clicked", async () => {
    render(component(mockConnection, [testFile]));

    const testArea = screen.getByText("Test Area");
    await userEvent.pointer({ keys: "[MouseRight]", target: testArea });

    const deleteButton = await screen.findByText("Delete");
    expect(deleteButton).toBeVisible();

    await userEvent.click(deleteButton);

    expect(await screen.findByRole("dialog")).toBeInTheDocument();
  });

  it("should handle delete confirmation in delete modal", async () => {
    render(component(mockConnection, [testFile]));

    const testArea = screen.getByText("Test Area");
    await userEvent.pointer({ keys: "[MouseRight]", target: testArea });

    const deleteButton = await screen.findByText("Delete");
    await userEvent.click(deleteButton);

    const dialog = await screen.findByRole("dialog");
    expect(dialog).toBeInTheDocument();

    const confirmButton = screen.getByRole("button", { name: "Delete" });
    await userEvent.click(confirmButton);

    expect(mockDeleteMutate).toHaveBeenCalledWith([testFile]);
  });

  it("should handle rename confirmation in rename modal", async () => {
    render(component(mockConnection, [testFile]));

    const testArea = screen.getByText("Test Area");
    await userEvent.pointer({ keys: "[MouseRight]", target: testArea });

    const renameButton = await screen.findByText("Rename");
    await userEvent.click(renameButton);

    const dialog = await screen.findByRole("dialog");
    expect(dialog).toBeInTheDocument();

    const input = screen.getByDisplayValue("test-file.txt");
    await userEvent.clear(input);
    await userEvent.type(input, "new-name.txt");

    const confirmButton = screen.getByRole("button", { name: "Rename" });
    await userEvent.click(confirmButton);

    expect(mockClipboardCut).toHaveBeenCalledWith([testFile]);
    expect(mockClipboardPaste).toHaveBeenCalledWith({
      destination: "/",
      newName: "new-name.txt",
    });
  });

  it.each([
    ["/parent/folder", "/parent/folder", "/parent"],
    ["/parent/folder", "/parent/folder/child", "/parent"],
    ["/parent/folder", "/unrelated", "/parent"],
    ["/parent/folder", "/parent", "/parent"],
    ["s3://bucket/parent/folder/", "s3://bucket/parent/folder/", "s3://bucket/parent"],
    ["s3://bucket/parent%20folder/folder", "s3://bucket/unrelated", "s3://bucket/parent%20folder"],
  ])("renames tree folder %s in its parent while browsing %s", async (resourceAddress, currentFolder, parent) => {
    const folder = createFolderListItem(resourceAddress);
    render(component({ ...mockConnection, currentFolder }, [folder]));

    await userEvent.pointer({ keys: "[MouseRight]", target: screen.getByText("Test Area") });
    await userEvent.click(await screen.findByText("Rename"));

    const input = await screen.findByDisplayValue(folder.name);
    await userEvent.clear(input);
    await userEvent.type(input, "renamed-folder");
    await userEvent.click(screen.getByRole("button", { name: "Rename" }));

    expect(mockClipboardCut).toHaveBeenCalledWith([folder]);
    expect(mockClipboardPaste).toHaveBeenCalledWith({ destination: parent, newName: "renamed-folder" });
  });

  it("should use fallback metadata when API fails", async () => {
    server.use(
      http.post("http://*/v1alpha/metadata/*", () => {
        return HttpResponse.error();
      })
    );

    render(component(mockConnection, [testFile]));

    const testArea = screen.getByText("Test Area");
    await userEvent.pointer({ keys: "[MouseRight]", target: testArea });

    const downloadButton = await screen.findByText("Download");
    expect(downloadButton).toBeVisible();
    expect(downloadButton.closest("button")).not.toBeDisabled();

    const copyButton = await screen.findByText("Copy");
    expect(copyButton).toBeVisible();
    expect(copyButton.closest("button")).not.toBeDisabled();

    const renameButton = await screen.findByText("Rename");
    expect(renameButton).toBeVisible();
    expect(renameButton.closest("button")).not.toBeDisabled();

    const cutButton = await screen.findByText("Cut");
    expect(cutButton).toBeVisible();
    expect(cutButton.closest("button")).not.toBeDisabled();

    const deleteButton = await screen.findByText("Delete");
    expect(deleteButton).toBeVisible();
    expect(deleteButton.closest("button")).not.toBeDisabled();
  });
});
