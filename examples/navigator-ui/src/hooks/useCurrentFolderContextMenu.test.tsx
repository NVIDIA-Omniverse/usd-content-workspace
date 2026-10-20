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
import { createFileListItem } from "@/state/path";
import { ClipboardOperation } from "@/state/slices/clipboardSlice";
import { Connection } from "@/state/slices/connectionSlice";
import { testConnection } from "@/test-utils/msw";
import { createTestQueryClient } from "@/test-utils/utils";
import { useCurrentFolderContextMenu } from "./useCurrentFolderContextMenu";

const mockUploadFiles = vi.fn();
const mockUploadFolder = vi.fn();
const mockNewFolderMutate = vi.fn();
const mockClipboardPaste = vi.fn();
const mockShowDirectoryPicker = vi.fn();
const mockShowOpenFilePicker = vi.fn();

vi.mock("@/hooks/useFileUpload", () => ({
  useFileUpload: () => ({
    uploadFiles: mockUploadFiles,
    uploadFolder: mockUploadFolder,
  }),
}));

vi.mock("@/hooks/useNewFolderMutation", () => ({
  useNewFolderMutation: () => ({
    mutateAsync: mockNewFolderMutate,
  }),
}));

const mockClipboardItems = [
  createFileListItem({
    resourceAddress: "/test-file.txt",
    resourceIdentity: "test-id",
    metadata: {
      dataObjectSize: 1024,
      lastModifiedTimestamp: "2023-01-01T00:00:00.000Z",
    },
  }),
];

const mockCanPaste = vi.fn().mockReturnValue(true);

vi.mock("@/hooks/usePathClipboard", () => ({
  usePathClipboard: () => ({
    empty: vi.fn().mockReturnValue(false),
    canPaste: mockCanPaste,
    items: mockClipboardItems,
    operation: ClipboardOperation.Copy,
    sourcePath: "/test-folder",
    clear: vi.fn(),
    copy: vi.fn(),
    cut: vi.fn(),
    paste: {
      mutate: mockClipboardPaste,
    } as any,
  }),
}));

vi.stubGlobal("showDirectoryPicker", mockShowDirectoryPicker);
vi.stubGlobal("showOpenFilePicker", mockShowOpenFilePicker);

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
      tags: {
        etag: "",
        value: {},
      },
    });
  })
);

beforeAll(() => server.listen());
afterEach(() => {
  server.resetHandlers();
  vi.clearAllMocks();
});
afterAll(() => server.close());

const TestComponent = ({ connection }: { connection: Connection }) => {
  const contextMenuHandler = useCurrentFolderContextMenu(connection);

  return (
    <div
      onContextMenu={(e) => {
        void contextMenuHandler(e);
      }}
      style={{ width: 200, height: 200, backgroundColor: "lightgray" }}
    >
      Test Area
    </div>
  );
};

const component = (connection: Connection) => {
  return (
    <QueryClientProvider client={createTestQueryClient()}>
      <MantineProvider>
        <ContextMenuProvider>
          <ModalsProvider>
            <TestComponent connection={connection} />
          </ModalsProvider>
        </ContextMenuProvider>
      </MantineProvider>
    </QueryClientProvider>
  );
};

describe("useCurrentFolderContextMenu", () => {
  const mockConnection: Connection = {
    ...testConnection,
    currentFolder: "/test-folder",
  };

  const rootConnection: Connection = testConnection;

  it("should not show context menu for top level address", async () => {
    render(component(rootConnection));

    const testArea = screen.getByText("Test Area");
    await userEvent.pointer({ keys: "[MouseRight]", target: testArea });

    expect(screen.queryByText("New folder")).not.toBeInTheDocument();
    expect(screen.queryByText("Upload files")).not.toBeInTheDocument();
  });

  it("should show context menu options", async () => {
    render(component(mockConnection));

    const testArea = screen.getByText("Test Area");
    await userEvent.pointer({ keys: "[MouseRight]", target: testArea });

    expect(await screen.findByText("New folder")).toBeVisible();
    expect(await screen.findByText("Upload files")).toBeVisible();
    expect(await screen.findByText("Upload folder")).toBeVisible();
  });

  it("should show paste option when clipboard is not empty", async () => {
    render(component(mockConnection));

    const testArea = screen.getByText("Test Area");
    await userEvent.pointer({ keys: "[MouseRight]", target: testArea });

    expect(await screen.findByText("Paste")).toBeVisible();
  });

  it("should disable write operations for read-only folders", async () => {
    server.use(
      http.post("http://*/v1alpha/metadata/*", () => {
        return HttpResponse.json({
          acl: {
            etag: "test-etag",
            value: ["read"],
          },
          tags: {
            etag: "",
            value: {},
          },
        });
      })
    );

    render(component(mockConnection));

    const testArea = screen.getByText("Test Area");
    await userEvent.pointer({ keys: "[MouseRight]", target: testArea });

    const newFolderButton = await screen.findByText("New folder");
    expect(newFolderButton).toBeVisible();
    expect(newFolderButton.closest("button")).toBeDisabled();

    const uploadFilesButton = await screen.findByText("Upload files");
    expect(uploadFilesButton).toBeVisible();
    expect(uploadFilesButton.closest("button")).toBeDisabled();

    const uploadFolderButton = await screen.findByText("Upload folder");
    expect(uploadFolderButton).toBeVisible();
    expect(uploadFolderButton.closest("button")).toBeDisabled();
  });

  it("should open copy modal when paste option is clicked", async () => {
    render(component(mockConnection));

    const testArea = screen.getByText("Test Area");
    await userEvent.pointer({ keys: "[MouseRight]", target: testArea });

    const pasteButton = await screen.findByText("Paste");
    expect(pasteButton).toBeVisible();

    await userEvent.click(pasteButton);

    expect(await screen.findByRole("dialog")).toBeInTheDocument();
  });

  it("should open new folder modal when new folder option is clicked", async () => {
    render(component(mockConnection));

    const testArea = screen.getByText("Test Area");
    await userEvent.pointer({ keys: "[MouseRight]", target: testArea });

    const newFolderButton = await screen.findByText("New folder");
    expect(newFolderButton).toBeVisible();

    await userEvent.click(newFolderButton);

    expect(await screen.findByRole("dialog")).toBeInTheDocument();
  });

  it("should handle upload folder operation click", async () => {
    const mockDirectoryHandle = {};
    mockShowDirectoryPicker.mockResolvedValue(mockDirectoryHandle);

    render(component(mockConnection));

    const testArea = screen.getByText("Test Area");
    await userEvent.pointer({ keys: "[MouseRight]", target: testArea });

    const uploadFolderButton = await screen.findByText("Upload folder");
    expect(uploadFolderButton).toBeVisible();

    await userEvent.click(uploadFolderButton);

    expect(mockShowDirectoryPicker).toHaveBeenCalledWith({ mode: "readwrite" });
    expect(mockUploadFolder).toHaveBeenCalledWith(mockDirectoryHandle);
  });

  it("should handle upload files operation click", async () => {
    const mockFileHandles = [{}, {}];
    mockShowOpenFilePicker.mockResolvedValue(mockFileHandles);

    render(component(mockConnection));

    const testArea = screen.getByText("Test Area");
    await userEvent.pointer({ keys: "[MouseRight]", target: testArea });

    const uploadFilesButton = await screen.findByText("Upload files");
    expect(uploadFilesButton).toBeVisible();

    await userEvent.click(uploadFilesButton);

    expect(mockShowOpenFilePicker).toHaveBeenCalledWith({ multiple: true });
    expect(mockUploadFiles).toHaveBeenCalledWith(mockFileHandles);
  });

  it("should use fallback metadata when API fails", async () => {
    server.use(
      http.post("http://*/v1alpha/metadata/*", () => {
        return HttpResponse.error();
      })
    );

    render(component(mockConnection));

    const testArea = screen.getByText("Test Area");
    await userEvent.pointer({ keys: "[MouseRight]", target: testArea });

    const newFolderButton = await screen.findByText("New folder");
    expect(newFolderButton).toBeVisible();
    expect(newFolderButton.closest("button")).not.toBeDisabled();

    const uploadFilesButton = await screen.findByText("Upload files");
    expect(uploadFilesButton).toBeVisible();
    expect(uploadFilesButton.closest("button")).not.toBeDisabled();

    const uploadFolderButton = await screen.findByText("Upload folder");
    expect(uploadFolderButton).toBeVisible();
    expect(uploadFolderButton.closest("button")).not.toBeDisabled();
  });

  it("should handle paste confirmation in copy modal", async () => {
    render(component(mockConnection));

    const testArea = screen.getByText("Test Area");
    await userEvent.pointer({ keys: "[MouseRight]", target: testArea });

    const pasteButton = await screen.findByText("Paste");
    expect(pasteButton).toBeVisible();

    await userEvent.click(pasteButton);

    const dialog = await screen.findByRole("dialog");
    expect(dialog).toBeInTheDocument();

    const confirmButton = screen.getByRole("button", { name: "Paste" });
    await userEvent.click(confirmButton);

    expect(mockClipboardPaste).toHaveBeenCalledWith({
      destination: "/test-folder",
      overwrite: true,
    });
  });

  it("should hide paste option when files would overwrite themselves", async () => {
    // Mock canPaste to return false (simulating file conflict scenario)
    mockCanPaste.mockReturnValue(false);

    render(component(mockConnection));

    const testArea = screen.getByText("Test Area");
    await userEvent.pointer({ keys: "[MouseRight]", target: testArea });

    // Wait for menu to appear
    await screen.findByText("New folder");

    // Paste option should not be visible when canPaste returns false
    expect(screen.queryByText("Paste")).not.toBeInTheDocument();

    // Reset mock
    mockCanPaste.mockReturnValue(true);
  });
});
