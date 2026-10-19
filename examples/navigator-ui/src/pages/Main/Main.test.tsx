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
import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { HTTPError } from "ky";
import { http } from "msw";
import { setupServer } from "msw/node";
import { InitialEntry, MemoryRouter, Route, Routes } from "react-router-dom";
import { afterEach, beforeEach, describe, it, vi } from "vitest";
import { Notifications, notifications } from "@mantine/notifications";
import { constructPathNavLink } from "@/hooks/usePathNavigation";
import { ListStatResult } from "@/services/storage";
import { useAppStore } from "@/state/appStore";
import { render } from "@/test-utils";
import { mockGetBoundingClientRect } from "@/test-utils/element";
import { createMockFile } from "@/test-utils/mockPath";
import { createTestHttpError, testConnection, testEndpoints } from "@/test-utils/msw";
import { createTestQueryClient } from "@/test-utils/utils";
import { Main } from "./Main";

const mocks = vi.hoisted(() => ({
  defaultDiscovery: "",
  bootstrap: {
    isPending: false,
    isError: false,
    isSuccess: true,
    error: null as unknown,
    fetchStatus: "idle" as "idle" | "fetching" | "paused",
    data: undefined as unknown,
  },
}));

vi.mock("@/hooks/useAppConfig", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/hooks/useAppConfig")>();
  return {
    ...actual,
    useAppConfig: () => ({ defaultDiscovery: mocks.defaultDiscovery, references: [] }),
  };
});

vi.mock("@/hooks/useDefaultServerBootstrap", () => ({
  useDefaultServerBootstrap: () => mocks.bootstrap,
}));

describe("Main", () => {
  const server = setupServer(...testEndpoints);

  beforeEach(() => {
    server.listen();

    mockGetBoundingClientRect();

    mocks.defaultDiscovery = "";
    mocks.bootstrap = {
      isPending: false,
      isError: false,
      isSuccess: true,
      error: null,
      fetchStatus: "idle",
      data: undefined,
    };

    act(() => {
      useAppStore.setState(useAppStore.getInitialState());
      useAppStore.setState({
        connections: {
          [testConnection.endpoint]: testConnection,
        },
        endpoints: new Set([testConnection.endpoint]),
        activeConnection: testConnection.endpoint,
      });
    });
  });

  afterEach(() => {
    server.restoreHandlers();
    vi.clearAllMocks();
  });

  afterAll(() => {
    server.close();
  });

  const component = ({ initialEntries = ["/"] }: { initialEntries?: InitialEntry[] } = {}) => (
    <MemoryRouter initialEntries={initialEntries}>
      <QueryClientProvider client={createTestQueryClient()}>
        <Routes>
          <Route path="/:endpoint/path/:path" Component={Main} />
          <Route path="/:endpoint/path/" Component={Main} />
          <Route path="/" Component={Main} />
        </Routes>
      </QueryClientProvider>
      <Notifications withinPortal={false} />
    </MemoryRouter>
  );

  it("initializes the connection", async () => {
    render(component());

    const connection = await screen.findByRole("treeitem", { name: "test-endpoint" });
    expect(connection).toBeVisible();
    expect(connection.ariaSelected).toBe("false");
    expect(connection.ariaExpanded).toBe("false");
  });

  it("initializes the connection from URL", async () => {
    render(
      component({
        initialEntries: [constructPathNavLink(testConnection.endpoint)],
      })
    );

    const connection = await screen.findByRole("treeitem", { name: "test-endpoint" });
    expect(connection).toBeVisible();
    await waitFor(() => expect(connection.ariaSelected).toBe("true"));
    await waitFor(() => expect(connection.ariaExpanded).toBe("true"));
  });

  it("initializes the folder from URL", async () => {
    render(
      component({
        initialEntries: [constructPathNavLink(testConnection.endpoint, "http://test-server/bucket1/folder1")],
      })
    );

    const folderNode = await screen.findByRole("treeitem", { name: "folder1" });
    await waitFor(() => expect(folderNode.ariaSelected).toBe("true"));
    await waitFor(() => expect(folderNode.ariaExpanded).toBe("true"));

    const bucketNode = await screen.findByRole("treeitem", { name: "bucket1" });
    await waitFor(() => expect(bucketNode.ariaExpanded).toBe("true"));

    const connectionNode = await screen.findByRole("treeitem", { name: "test-endpoint" });
    await waitFor(() => expect(connectionNode.ariaExpanded).toBe("true"));
  });

  it("initializes the folder from a URL without base path", async () => {
    render(
      component({
        initialEntries: [constructPathNavLink(testConnection.endpoint, "omniverse://test/folder/")],
      })
    );

    const folderNode = await screen.findByRole("treeitem", { name: "folder" });
    await waitFor(() => expect(folderNode.ariaSelected).toBe("true"));
    await waitFor(() => expect(folderNode.ariaExpanded).toBe("true"));

    const bucketNode = await screen.findByRole("treeitem", { name: "test" });
    await waitFor(() => expect(bucketNode.ariaExpanded).toBe("true"));

    const connectionNode = await screen.findByRole("treeitem", { name: "test-endpoint" });
    await waitFor(() => expect(connectionNode.ariaExpanded).toBe("true"));
  });

  const setupDuplicateAddresses = () => {
    server.use(
      http.get("http://*/v1alpha/capabilities/top-level-addresses", () => {
        const body = {
          items: [
            { topLevelAddress: "http://test-server/ovc/" },
            { topLevelAddress: "http://test-server/ovc/nvidia/" },
          ],
        };
        return new Response(JSON.stringify(body));
      }),
      http.get("http://*/v1alpha/filefolder/liststat/:resourceAddress", ({ params }) => {
        const resourceAddress = decodeURI(params.resourceAddress as string);
        const subfolders: Record<string, string[]> = {
          "http://test-server/ovc/": ["http://test-server/ovc/nvidia/"],
          "http://test-server/ovc/nvidia/": ["http://test-server/ovc/nvidia/models/"],
          "http://test-server/ovc/nvidia/models/": [],
        };
        if (resourceAddress in subfolders) {
          const body = {
            folderAddress: resourceAddress,
            isRoot: false,
            entries: [],
            subfolderAddresses: subfolders[resourceAddress],
            nextContinuationHandle: null,
          } as ListStatResult;
          return new Response(JSON.stringify(body), { status: 200 });
        }
        return new Response(null, { status: 404 });
      })
    );
  };

  it("keeps the clicked top-level address selected when the same address is also nested elsewhere", async () => {
    setupDuplicateAddresses();

    render(component());

    const connection = await screen.findByRole("treeitem", { name: "test-endpoint" });
    await userEvent.click(connection);

    const nvidia = await screen.findByRole("treeitem", { name: "nvidia" });
    await userEvent.click(nvidia);

    await waitFor(() => expect(nvidia.ariaSelected).toBe("true"));

    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(screen.getAllByRole("treeitem", { name: "nvidia" })).toHaveLength(1);
    expect(nvidia.ariaSelected).toBe("true");
    expect(screen.getByRole("treeitem", { name: "ovc" }).ariaExpanded).toBe("false");
  });

  it("resolves a deep path under the most specific top-level address", async () => {
    setupDuplicateAddresses();

    render(
      component({
        initialEntries: [constructPathNavLink(testConnection.endpoint, "http://test-server/ovc/nvidia/models/")],
      })
    );

    const models = await screen.findByRole("treeitem", { name: "models" });
    await waitFor(() => expect(models.ariaSelected).toBe("true"));

    const nvidiaNodes = screen.getAllByRole("treeitem", { name: "nvidia" });
    expect(nvidiaNodes).toHaveLength(1);
    await waitFor(() => expect(nvidiaNodes[0].ariaExpanded).toBe("true"));

    expect(screen.getByRole("treeitem", { name: "ovc" }).ariaExpanded).toBe("false");
  });

  it("displays an error if URL contains an unknown endpoint", async () => {
    const showSpy = vi.spyOn(notifications, "show");
    render(
      component({
        initialEntries: [constructPathNavLink("http://test-invalid", "http://test-server/bucket1/folder1")],
      })
    );

    await waitFor(() => {
      expect(showSpy).toHaveBeenCalledWith(
        expect.objectContaining({
          title: "Cannot connect to http://test-invalid",
          message: "http://test-invalid is not added to the application.",
          type: "error",
        }),
        undefined
      );
    });
  });

  it("renders path panel if one path is selected", async () => {
    render(component());

    act(() => {
      useAppStore.setState({
        connections: {
          [testConnection.endpoint]: { ...testConnection, selectedPaths: new Set([createMockFile("test.txt")]) },
        },
      });
    });

    const aside = await screen.findByRole("complementary");
    expect(within(aside).getByRole("heading", { name: "test.txt" })).toBeVisible();
  });

  it("renders path selection panel if multiple paths are selected", async () => {
    render(component());

    act(() => {
      useAppStore.setState({
        connections: {
          [testConnection.endpoint]: {
            ...testConnection,
            selectedPaths: new Set([createMockFile("test.txt"), createMockFile("image.png")]),
          },
        },
      });
    });

    const aside = await screen.findByRole("complementary");
    expect(within(aside).getByRole("heading", { name: "2 paths selected" })).toBeVisible();
  });

  it("renders current folder panel", async () => {
    render(component());

    act(() => {
      useAppStore.setState({
        connections: {
          [testConnection.endpoint]: {
            ...testConnection,
            selectedPaths: new Set(),
          },
        },
      });
    });

    const aside = await screen.findByRole("complementary");
    expect(within(aside).getByRole("heading", { name: "test-endpoint" })).toBeVisible();
  });

  describe("default-server bootstrap loading", () => {
    beforeEach(() => {
      act(() => {
        useAppStore.setState({
          connections: {},
          endpoints: new Set(),
          activeConnection: "",
        });
      });
    });

    it("shows a centered spinner instead of the empty state while the bootstrap is pending", () => {
      mocks.defaultDiscovery = "https://stack.example/";
      mocks.bootstrap = {
        isPending: true,
        isError: false,
        isSuccess: false,
        error: null,
        fetchStatus: "fetching",
        data: undefined,
      };

      render(component());

      expect(screen.getByLabelText("Connecting to default server")).toBeVisible();
      expect(screen.queryByText("No active connection.")).toBeNull();
      expect(screen.queryByRole("tree", { name: "Path tree" })).toBeNull();
    });

    it("renders the layout once the bootstrap completes", async () => {
      mocks.defaultDiscovery = "https://stack.example/";
      act(() => {
        useAppStore.setState({
          connections: { [testConnection.endpoint]: testConnection },
          endpoints: new Set([testConnection.endpoint]),
          activeConnection: testConnection.endpoint,
        });
      });

      render(component({ initialEntries: [constructPathNavLink(testConnection.endpoint)] }));

      expect(screen.queryByLabelText("Connecting to default server")).toBeNull();
      const connection = await screen.findByRole("treeitem", { name: "test-endpoint" });
      expect(connection).toBeVisible();
    });

    it("keeps the spinner visible without a notification on a 401 from discovery", () => {
      const showSpy = vi.spyOn(notifications, "show");
      mocks.defaultDiscovery = "https://stack.example/";
      const httpError: HTTPError = createTestHttpError({ status: 401 });
      mocks.bootstrap = {
        isPending: false,
        isError: true,
        isSuccess: false,
        error: httpError,
        fetchStatus: "idle",
        data: undefined,
      };

      render(component());

      expect(screen.getByLabelText("Connecting to default server")).toBeVisible();
      expect(screen.queryByText("No active connection.")).toBeNull();
      expect(showSpy).not.toHaveBeenCalled();
    });

    it("surfaces a non-auth bootstrap failure as an error notification", async () => {
      const showSpy = vi.spyOn(notifications, "show");
      mocks.defaultDiscovery = "https://stack.example/";
      mocks.bootstrap = {
        isPending: false,
        isError: true,
        isSuccess: false,
        error: new Error("network down"),
        fetchStatus: "idle",
        data: undefined,
      };

      render(component());

      await waitFor(() =>
        expect(showSpy).toHaveBeenCalledWith(
          expect.objectContaining({
            title: "Failed to connect to https://stack.example/",
            message: "network down",
            type: "error",
          }),
          undefined
        )
      );
    });
  });
});
