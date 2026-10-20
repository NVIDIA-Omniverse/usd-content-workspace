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
import { beforeEach, describe, expect, it, vi } from "vitest";
import { MantineProvider } from "@mantine/core";
import { notifications, Notifications } from "@mantine/notifications";
import { Connection } from "@/state/appStore";
import { createMockFile } from "@/test-utils/mockPath";
import { testConnection } from "@/test-utils/msw";
import { useArchiveDownload } from "./useArchiveDownload";

const mockGetApi = vi.fn();
const mockPrepare = vi.fn();
const mockTriggerFormDownload = vi.fn<(params: unknown) => void>();
const mockIsUsable = vi.fn<(url: string) => boolean>();

vi.mock("./useServiceApi", () => ({
  useServiceApi: () => ({ getApi: mockGetApi, createServiceApi: vi.fn() }),
}));

vi.mock("./useAppConfig", () => ({
  useAppConfig: () => ({ requestTimeout: 30000, batchSize: 25 }),
}));

const mockCreate = vi.fn((_url: string, _options: unknown) => ({}));

vi.mock("@/services/api", () => ({
  create: (url: string, options: unknown) => mockCreate(url, options),
  getApiOptions: (connection: Connection) => ({ endpoint: connection.endpoint }),
}));

vi.mock("@/services/archiveService", () => ({
  archiveService: () => ({ prepare: mockPrepare }),
  triggerFormDownload: (params: unknown) => mockTriggerFormDownload(params),
  isArchiveServiceUsable: (url: string) => mockIsUsable(url),
  toArchiveItems: (paths: { isFolderLike: boolean; resourceAddress: string }[]) =>
    paths.map((path) => ({ type: path.isFolderLike ? "folder" : "file", address: path.resourceAddress })),
}));

const fakeApi = { url: "https://archive.test/v1alpha/", baseUrl: "https://archive.test", versionPath: "/v1alpha/" };

const wrapper = ({ children }: { children: ReactNode }) => (
  <MantineProvider>
    {children}
    <Notifications />
  </MantineProvider>
);

describe("useArchiveDownload", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    notifications.clean();
    mockGetApi.mockResolvedValue(fakeApi);
    mockIsUsable.mockReturnValue(true);
    mockPrepare.mockResolvedValue({ downloadAction: "/v1alpha/archive/download", expiresAt: "2026-01-01T00:00:00Z" });
  });

  it("is available when discovery advertises a same-site archive service", async () => {
    const { result } = renderHook(() => useArchiveDownload(testConnection), { wrapper });
    await waitFor(() => expect(result.current.available).toBe(true));
  });

  it("is unavailable without a connection", async () => {
    const { result } = renderHook(() => useArchiveDownload(undefined), { wrapper });
    await waitFor(() => expect(result.current.available).toBe(false));
    expect(mockGetApi).not.toHaveBeenCalled();
  });

  it("resolves the archive service and sends requests through the given connection", async () => {
    const otherConnection: Connection = {
      ...testConnection,
      endpoint: "http://other-endpoint",
      info: { auth: null, discovery: "http://other-discovery" },
    };
    const { result } = renderHook(() => useArchiveDownload(otherConnection), { wrapper });
    await waitFor(() => expect(result.current.available).toBe(true));

    act(() => result.current.download([createMockFile("a/file.txt")]));

    expect(mockGetApi).toHaveBeenCalledWith("http://other-discovery", "archive", { endpoint: "http://other-endpoint" });
    expect(mockCreate).toHaveBeenCalledWith(fakeApi.url, { endpoint: "http://other-endpoint" });
  });

  it("re-resolves the archive service when the connection changes", async () => {
    const otherConnection: Connection = {
      ...testConnection,
      endpoint: "http://other-endpoint",
      info: { auth: null, discovery: "http://other-discovery" },
    };
    const { result, rerender } = renderHook(({ connection }) => useArchiveDownload(connection), {
      wrapper,
      initialProps: { connection: testConnection },
    });
    await waitFor(() => expect(result.current.available).toBe(true));

    rerender({ connection: otherConnection });

    await waitFor(() =>
      expect(mockGetApi).toHaveBeenLastCalledWith("http://other-discovery", "archive", {
        endpoint: "http://other-endpoint",
      })
    );
  });

  it("is unavailable when the archive service is not advertised", async () => {
    mockGetApi.mockResolvedValue(null);
    const { result } = renderHook(() => useArchiveDownload(testConnection), { wrapper });
    await waitFor(() => expect(mockGetApi).toHaveBeenCalled());
    expect(result.current.available).toBe(false);
  });

  it("is unavailable when the archive service is on a different origin", async () => {
    mockIsUsable.mockReturnValue(false);
    const { result } = renderHook(() => useArchiveDownload(testConnection), { wrapper });
    await waitFor(() => expect(mockIsUsable).toHaveBeenCalled());
    expect(result.current.available).toBe(false);
  });

  it("starts the native download as soon as preparation finishes", async () => {
    const { result } = renderHook(() => useArchiveDownload(testConnection), { wrapper });
    await waitFor(() => expect(result.current.available).toBe(true));

    act(() => result.current.download([createMockFile("a/file.txt")]));

    expect(await screen.findByText(/Download started/)).toBeVisible();
    expect(mockTriggerFormDownload).toHaveBeenCalledTimes(1);
    expect(mockTriggerFormDownload).toHaveBeenCalledWith({
      action: "https://archive.test/v1alpha/archive/download",
      items: [{ type: "file", address: "a/file.txt" }],
      archiveName: "file.txt",
    });
  });

  it("re-triggers the native download when the manual link is clicked", async () => {
    const { result } = renderHook(() => useArchiveDownload(testConnection), { wrapper });
    await waitFor(() => expect(result.current.available).toBe(true));

    act(() => result.current.download([createMockFile("a/file.txt")]));

    const link = await screen.findByText("click here");
    await userEvent.click(link);

    expect(mockTriggerFormDownload).toHaveBeenCalledTimes(2);
  });

  it("cancels the preparation when the cancel button is clicked", async () => {
    mockPrepare.mockReturnValue(new Promise<never>(() => undefined));
    const { result } = renderHook(() => useArchiveDownload(testConnection), { wrapper });
    await waitFor(() => expect(result.current.available).toBe(true));

    act(() => result.current.download([createMockFile("a/file.txt")]));

    expect(await screen.findByText("Preparing archive…")).toBeVisible();

    const cancelButton = await screen.findByRole("button", { name: "Cancel" });
    await userEvent.click(cancelButton);

    expect(await screen.findByText("Cancelled.")).toBeVisible();
    expect(mockTriggerFormDownload).not.toHaveBeenCalled();
  });

  it("shows the failure detail when preparation fails", async () => {
    mockPrepare.mockRejectedValue(new Error("too many items (max 10000)"));
    const { result } = renderHook(() => useArchiveDownload(testConnection), { wrapper });
    await waitFor(() => expect(result.current.available).toBe(true));

    act(() => result.current.download([createMockFile("a/file.txt")]));

    expect(await screen.findByText("too many items (max 10000)")).toBeVisible();
    expect(mockTriggerFormDownload).not.toHaveBeenCalled();
  });
});
