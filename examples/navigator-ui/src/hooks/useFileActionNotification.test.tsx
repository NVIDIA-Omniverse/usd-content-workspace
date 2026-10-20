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

import { ReactNode } from "react";
import { act, renderHook, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { MantineProvider } from "@mantine/core";
import { notifications, Notifications } from "@mantine/notifications";
import { createMockFile, createMockFolder } from "@/test-utils/mockPath";
import { useFileActionNotification } from "./useFileActionNotification";

const wrapper = ({ children }: { children: ReactNode }) => (
  <MantineProvider>
    {children}
    <Notifications />
  </MantineProvider>
);

describe("useFileActionNotification", () => {
  beforeEach(() => {
    notifications.clean();
  });

  it("should display a notification when deleting a single file", async () => {
    const resources = [createMockFile("test-file.txt")];

    const { result } = renderHook(() => useFileActionNotification(), { wrapper });
    act(() => {
      result.current({ action: "Delete", data: resources });
    });

    const title = await screen.findByText("Delete test-file.txt");
    expect(title).toBeVisible();

    const description = await screen.findByText("Starting...");
    expect(description).toBeVisible();
  });

  it("should display a notification with plural title when deleting multiple files", async () => {
    const resources = [createMockFile("file1.txt"), createMockFile("file2.txt")];

    const { result } = renderHook(() => useFileActionNotification(), { wrapper });
    act(() => {
      result.current({ action: "Delete", data: resources });
    });

    const title = await screen.findByText("Delete 2 items");
    expect(title).toBeVisible();
  });

  it("should update notification with progress text", async () => {
    const resources = [createMockFile("file-1.txt")];

    const { result } = renderHook(() => useFileActionNotification(), { wrapper });
    act(() => {
      const notification = result.current({ action: "Delete", data: resources });
      notification.update("file-2.txt");
    });

    const description = await screen.findByText("file-2.txt");
    expect(description).toBeVisible();
  });

  it("should update notification on successful completion of a single file", async () => {
    const resources = [createMockFile("test-file.txt")];

    const { result } = renderHook(() => useFileActionNotification(), { wrapper });
    act(() => {
      const notification = result.current({ action: "Delete", data: resources });
      notification.complete(["test-file.txt"]);
    });

    const title = await screen.findByText("Delete test-file.txt");
    expect(title).toBeVisible();

    const description = await screen.findByText("Success.");
    expect(description).toBeVisible();
  });

  it("should update notification on successful completion of multiple files", async () => {
    const { result } = renderHook(() => useFileActionNotification(), { wrapper });
    act(() => {
      const notification = result.current({ action: "Delete", data: [createMockFolder("documents")] });
      notification.complete(["documents/file1.txt", "documents/file2.txt", "documents/file3.txt"]);
    });

    const title = await screen.findByText("Delete documents");
    expect(title).toBeVisible();

    const description = await screen.findByText("Success: 3 items.");
    expect(description).toBeVisible();
  });

  it("should update notification on partial completion", async () => {
    const resources = [createMockFile("file1.txt"), createMockFile("file2.txt")];

    const { result } = renderHook(() => useFileActionNotification(), { wrapper });
    act(() => {
      const notification = result.current({ action: "Delete", data: resources });
      notification.completePartially(["file1.txt"], ["file2.txt"]);
    });

    const title = await screen.findByText("Delete 2 items");
    expect(title).toBeVisible();

    const description = await screen.findByText("Completed partially: 1 completed, 1 failed.");
    expect(description).toBeVisible();
  });

  it("should update notification on partial completion with skipped files", async () => {
    const resources = [createMockFile("file1.txt"), createMockFile("file2.txt"), createMockFile("file3.txt")];

    const { result } = renderHook(() => useFileActionNotification(), { wrapper });
    act(() => {
      const notification = result.current({ action: "Copy", data: resources });
      notification.completePartially(["file1.txt"], ["file2.txt"], ["/source/file3.txt"]);
    });

    const title = await screen.findByText("Copy 3 items");
    expect(title).toBeVisible();

    const description = await screen.findByText("Completed partially: 1 completed, 1 skipped, 1 failed.");
    expect(description).toBeVisible();
  });

  it("should treat completion with skipped files (and no failures) as success, not a warning", async () => {
    const resources = [createMockFile("file1.txt"), createMockFile("file2.txt")];

    const { result } = renderHook(() => useFileActionNotification(), { wrapper });
    act(() => {
      const notification = result.current({ action: "Copy", data: resources });
      notification.complete(["file1.txt"], ["file2.txt"]);
    });

    const description = await screen.findByText("Success: 1 completed, 1 skipped.");
    expect(description).toBeVisible();
  });

  it("should update notification on failure of a single file", async () => {
    const resources = [createMockFile("test-file.txt")];

    const { result } = renderHook(() => useFileActionNotification(), { wrapper });

    act(() => {
      const notification = result.current({ action: "Delete", data: resources });
      notification.fail(["test-file.txt"]);
    });

    const title = await screen.findByText("Delete test-file.txt");
    expect(title).toBeVisible();

    const description = await screen.findByText("Failed to delete test-file.txt.");
    expect(description).toBeVisible();
  });

  it("should update notification on failure of multiple files", async () => {
    const { result } = renderHook(() => useFileActionNotification(), { wrapper });

    act(() => {
      const notification = result.current({ action: "Delete", data: [createMockFolder("documents")] });
      notification.fail(["documents/file1.txt", "documents/file2.txt", "documents/file3.txt"]);
    });

    const title = await screen.findByText("Delete documents");
    expect(title).toBeVisible();

    const description = await screen.findByText("Failed to delete 3 items.");
    expect(description).toBeVisible();
  });

  it("should abort deletion when cancel button is clicked", async () => {
    const resources = [createMockFile("test-file.txt")];

    const { result } = renderHook(() => useFileActionNotification(), { wrapper });
    let notification!: ReturnType<typeof result.current>;
    act(() => {
      notification = result.current({ action: "Delete", data: resources });
    });

    const cancelButton = await screen.findByRole("button", { name: "Cancel" });
    expect(cancelButton).toBeVisible();

    const user = userEvent.setup();
    await user.click(cancelButton);

    await waitFor(() => {
      expect(notification.signal.aborted).toBe(true);
    });
  });
});
