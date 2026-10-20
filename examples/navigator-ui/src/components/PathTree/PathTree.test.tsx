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
import { QueryClientProvider } from "@tanstack/react-query";
import { act, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { setupServer } from "msw/node";
import { BrowserRouter } from "react-router-dom";
import { afterEach, beforeEach, describe } from "vitest";
import { Notifications } from "@mantine/notifications";
import { DiscoveredApi } from "@/hooks/useServiceApi";
import { useStorageSubscription } from "@/hooks/useStorageSubscription";
import { DiscoveryAuthConfig } from "@/services/auth";
import { OmniEvent } from "@/services/notificationService";
import { ListStatResult } from "@/services/storage";
import { useAppStore } from "@/state/appStore";
import { render } from "@/test-utils";
import { mockGetBoundingClientRect } from "@/test-utils/element";
import { createTestHttpError, testConnection, testEndpoints } from "@/test-utils/msw";
import { createTestAuth, createTestQueryClient } from "@/test-utils/utils";
import { PathTree } from "./PathTree";

const mocks = vi.hoisted(() => {
  const authConfig: DiscoveryAuthConfig = {
    openidConfiguration: "",
    clients: {},
  };

  return {
    authorize: vi.fn(),
    logout: vi.fn(),
    authConfig,
    getAuthConfig: vi.fn().mockResolvedValue(authConfig),
    getApi: vi.fn(
      (_name: string): Promise<DiscoveredApi | null> =>
        Promise.resolve({
          url: "http://test-endpoint/v1alpha",
          baseUrl: "http://test-endpoint",
          versionPath: "/v1alpha",
          version: "v1alpha",
        })
    ),
    defaultDiscovery: "",
  };
});

vi.mock("@/services/auth", async () => {
  const module = await vi.importActual<typeof import("@/services/auth")>("@/services/auth");
  return {
    ...module,
    authorize: mocks.authorize,
    logout: mocks.logout,
    getAuthConfig: mocks.getAuthConfig,
  };
});

vi.mock("@/hooks/useServiceApi", async () => {
  const { create } = await vi.importActual<typeof import("@/services/api")>("@/services/api");
  return {
    useServiceApi: () => ({
      getApi: (_discovery: string, name: string) => mocks.getApi(name),
      createServiceApi: async (_discovery: string, name: string, options?: import("@/services/api").ApiOptions) => {
        const api = await mocks.getApi(name);
        return api ? create(api.url, options) : null;
      },
    }),
  };
});

vi.mock("@/hooks/useAppConfig", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/hooks/useAppConfig")>();
  return {
    ...actual,
    useAppConfig: () => ({ defaultDiscovery: mocks.defaultDiscovery }),
  };
});

describe("PathTree", () => {
  const server = setupServer(...testEndpoints);

  beforeEach(() => {
    act(() => {
      useAppStore.setState(useAppStore.getInitialState());
    });

    server.listen();
    mockGetBoundingClientRect((element) => element.getAttribute("aria-label") === "Path tree");
    mocks.defaultDiscovery = "";
  });

  afterEach(() => {
    server.resetHandlers(...testEndpoints);
    mocks.authorize.mockClear();
    mocks.logout.mockClear();
    mocks.getApi.mockClear();
  });

  afterAll(() => {
    server.close();
  });

  const component = (children?: ReactNode) => (
    <QueryClientProvider client={createTestQueryClient()}>
      <BrowserRouter>
        <PathTree />
        {children}
      </BrowserRouter>
      <Notifications withinPortal={false} />
    </QueryClientProvider>
  );

  it("renders the tree root", async () => {
    render(component());
    const root = await screen.findByRole("treeitem", { name: "Omniverse Storage" });
    expect(root).toBeVisible();
  });

  it("adds new connection with authentication", async () => {
    const httpError = createTestHttpError({ status: 401 });
    mocks.getApi.mockRejectedValueOnce(httpError);

    render(component(), { modalProps: { withinPortal: false } });

    const connectionControl = await screen.findByRole("treeitem", { name: "Add Connection" });
    expect(connectionControl).toBeVisible();
    await userEvent.click(connectionControl);

    const connectionInput = await screen.findByLabelText("Discovery Hostname");
    await userEvent.type(connectionInput, "test-discovery");

    const connect = await screen.findByRole("button", { name: "Connect" });
    await userEvent.click(connect);

    await waitFor(() =>
      expect(mocks.authorize).toBeCalledWith({ urls: { discovery: "test-discovery" } }, mocks.authConfig)
    );
  });

  it("adds new connection without authentication", async () => {
    const storageApi: DiscoveredApi = {
      url: "http://test-storage/v1alpha",
      baseUrl: "http://test-storage/",
      versionPath: "/v1alpha",
      version: "v1alpha",
    };
    mocks.getApi.mockResolvedValueOnce(storageApi);

    render(component(), { modalProps: { withinPortal: false } });

    const connectionControl = await screen.findByRole("treeitem", { name: "Add Connection" });
    await userEvent.click(connectionControl);

    const connectionInput = await screen.findByLabelText("Discovery Hostname");
    await userEvent.type(connectionInput, "test-discovery");

    const connect = await screen.findByRole("button", { name: "Connect" });
    await userEvent.click(connect);

    const newConnection = await screen.findByRole("treeitem", { name: "test-storage" });
    expect(newConnection).toBeVisible();
  });

  it("fails to add new connection if no REST endpoints are available", async () => {
    mocks.getApi.mockResolvedValueOnce(null);

    render(component(), { modalProps: { withinPortal: false } });

    const connectionControl = await screen.findByRole("treeitem", { name: "Add Connection" });
    await userEvent.click(connectionControl);

    const connectionInput = await screen.findByLabelText("Discovery Hostname");
    await userEvent.type(connectionInput, "test-discovery");

    const connect = await screen.findByRole("button", { name: "Connect" });
    await userEvent.click(connect);

    const error = await screen.findByText(/This deployment does not have any Storage API available for Navigator./);
    expect(error).toBeVisible();
  });

  it("fails to add new connection if auth is required but auth config is not available", async () => {
    mocks.getApi.mockRejectedValueOnce(createTestHttpError({ status: 401 }));
    mocks.getAuthConfig.mockResolvedValueOnce(null);

    render(component(), { modalProps: { withinPortal: false } });

    const connectionControl = await screen.findByRole("treeitem", { name: "Add Connection" });
    await userEvent.click(connectionControl);

    const connectionInput = await screen.findByLabelText("Discovery Hostname");
    await userEvent.type(connectionInput, "test-discovery");

    const connect = await screen.findByRole("button", { name: "Connect" });
    await userEvent.click(connect);

    const error = await screen.findByText(/requires authentication but authentication configuration is not available/);
    expect(error).toBeVisible();
  });

  it("renders connections", async () => {
    act(() => {
      useAppStore.setState({
        endpoints: new Set(["http://test-endpoint/"]),
      });
    });

    render(component());

    const endpoint = await screen.findByRole("treeitem", { name: "test-endpoint" });
    expect(endpoint).toBeVisible();

    act(() => {
      useAppStore.setState({
        endpoints: new Set(["http://test-endpoint/", "http://test-endpoint-2/"]),
      });
    });

    const endpoint2 = await screen.findByRole("treeitem", { name: "test-endpoint-2" });
    expect(endpoint2).toBeVisible();
  });

  it("shows the current user email next to the connection name", async () => {
    const claims = btoa(JSON.stringify({ email: "user@nvidia.com" }))
      .replace(/\+/g, "-")
      .replace(/\//g, "_")
      .replace(/=/g, "");
    const idToken = `header.${claims}.signature`;

    act(() => {
      useAppStore.setState({
        endpoints: new Set(["http://test-endpoint/"]),
        connections: {
          "http://test-endpoint/": {
            ...testConnection,
            endpoint: "http://test-endpoint/",
            info: {
              discovery: "http://discovery",
              auth: { ...createTestAuth(), idToken },
            },
          },
        },
      });
    });

    render(component());

    expect(await screen.findByText("(user@nvidia.com)")).toBeVisible();
    const connection = await screen.findByRole("treeitem", { name: /test-endpoint/ });
    expect(connection).toHaveTextContent("(user@nvidia.com)");
  });

  it("deletes connections", async () => {
    act(() => {
      useAppStore.setState({
        endpoints: new Set(["http://test-endpoint/"]),
      });
    });

    render(component());

    const endpoint = await screen.findByRole("treeitem", { name: "test-endpoint" });
    await userEvent.pointer({ keys: "[MouseRight>]", target: endpoint });

    const deleteMenuItem = await screen.findByText("Delete");
    await userEvent.click(deleteMenuItem);

    expect(screen.queryByRole("treeitem", { name: "test-endpoint" })).toBeNull();
  });

  it("renders top-level addresses", async () => {
    act(() => {
      useAppStore.setState({
        endpoints: new Set(["http://test-endpoint/"]),
        connections: {
          "http://test-endpoint/": testConnection,
        },
      });
    });

    render(component());

    const endpoint = await screen.findByRole("treeitem", { name: "test-endpoint" });
    await userEvent.click(endpoint);
    expect(endpoint.ariaExpanded).toBe("true");
    expect(endpoint.ariaSelected).toBe("true");

    const bucket1 = await screen.findByRole("treeitem", { name: "bucket1" });
    expect(bucket1).toBeVisible();

    const bucket2 = await screen.findByRole("treeitem", { name: "bucket2" });
    expect(bucket2).toBeVisible();
  });

  it("renders folders", async () => {
    act(() => {
      useAppStore.setState({
        endpoints: new Set(["http://test-endpoint/"]),
        connections: {
          "http://test-endpoint/": testConnection,
        },
      });
    });

    render(component());

    const endpoint = await screen.findByRole("treeitem", { name: "test-endpoint" });
    await userEvent.click(endpoint);

    const bucket2 = await screen.findByRole("treeitem", { name: "bucket2" });
    await userEvent.click(bucket2);
    expect(bucket2.ariaExpanded).toBe("true");
    expect(bucket2.ariaSelected).toBe("true");

    const folder3 = await screen.findByRole("treeitem", { name: "folder3" });
    expect(folder3).toBeVisible();
    await userEvent.click(folder3);
    expect(folder3.ariaExpanded).toBe("true");
    expect(folder3.ariaSelected).toBe("true");

    const subfolder1 = await screen.findByRole("treeitem", { name: "subfolder1" });
    expect(subfolder1).toBeVisible();
  });

  it("expands a collapsed folder when its chevron is clicked", async () => {
    act(() => {
      useAppStore.setState({
        endpoints: new Set(["http://test-endpoint/"]),
        connections: {
          "http://test-endpoint/": testConnection,
        },
      });
    });

    render(component());

    const endpoint = await screen.findByRole("treeitem", { name: "test-endpoint" });
    const endpointChevron = endpoint.querySelector("[data-tree-chevron]");
    expect(endpointChevron).not.toBeNull();
    await userEvent.click(endpointChevron!);

    expect(endpoint.ariaExpanded).toBe("true");
    const bucket2 = await screen.findByRole("treeitem", { name: "bucket2" });
    expect(bucket2).toBeVisible();
  });

  it("collapses an expanded folder when its chevron is clicked without re-expanding it", async () => {
    act(() => {
      useAppStore.setState({
        endpoints: new Set(["http://test-endpoint/"]),
        connections: {
          "http://test-endpoint/": testConnection,
        },
      });
    });

    render(component());

    const endpoint = await screen.findByRole("treeitem", { name: "test-endpoint" });
    await userEvent.click(endpoint);

    const bucket2 = await screen.findByRole("treeitem", { name: "bucket2" });
    await userEvent.click(bucket2);
    expect(bucket2.ariaExpanded).toBe("true");

    const folder3 = await screen.findByRole("treeitem", { name: "folder3" });
    await userEvent.click(folder3);
    expect(folder3.ariaExpanded).toBe("true");

    const bucket2Chevron = bucket2.querySelector("[data-tree-chevron]");
    expect(bucket2Chevron).not.toBeNull();
    await userEvent.click(bucket2Chevron!);

    await waitFor(() => expect(bucket2.ariaExpanded).toBe("false"));
    // Guard against the navigation-driven re-expand regression.
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(bucket2.ariaExpanded).toBe("false");
  });

  it("renders folders with percent signs in the name", async () => {
    server.use(
      http.get("http://*/v1alpha/filefolder/liststat/:resourceAddress", ({ params }) => {
        const resourceAddress = decodeURI(params.resourceAddress as string);
        if (resourceAddress === `http://test-server/bucket2/`) {
          const body = {
            folderAddress: resourceAddress,
            isRoot: true,
            entries: [],
            subfolderAddresses: [`http://test-server/bucket2/percent%folder/`],
            nextContinuationHandle: null,
          } as ListStatResult;
          return new Response(JSON.stringify(body), { status: 200 });
        } else if (resourceAddress === `http://test-server/bucket2/percent%folder/`) {
          const body = {
            folderAddress: resourceAddress,
            isTopLevelAddress: true,
            entries: [],
            subfolderAddresses: [],
            nextContinuationHandle: null,
            isRoot: false,
          } as ListStatResult;
          return new Response(JSON.stringify(body), { status: 200 });
        }
        return new Response(null, { status: 404 });
      })
    );

    act(() => {
      useAppStore.setState({
        endpoints: new Set(["http://test-endpoint/"]),
        connections: {
          "http://test-endpoint/": testConnection,
        },
      });
    });

    render(component());

    const endpoint = await screen.findByRole("treeitem", { name: "test-endpoint" });
    await userEvent.click(endpoint);

    const bucket2 = await screen.findByRole("treeitem", { name: "bucket2" });
    await userEvent.click(bucket2);

    const percentFolder = await screen.findByRole("treeitem", { name: "percent%folder" });
    expect(percentFolder).toBeVisible();
  });

  it("renders the same folder from different connections", async () => {
    act(() => {
      useAppStore.setState({
        endpoints: new Set(["http://test-endpoint/", "http://test-endpoint-2/"]),
        connections: {
          "http://test-endpoint/": testConnection,
          "http://test-endpoint-2/": {
            ...testConnection,
            endpoint: "http://test-endpoint-2/",
          },
        },
      });
    });

    render(component());

    const endpoint1 = await screen.findByRole("treeitem", { name: "test-endpoint" });
    await userEvent.click(endpoint1);

    const endpoint2 = await screen.findByRole("treeitem", { name: "test-endpoint-2" });
    await userEvent.click(endpoint2);

    const buckets = await screen.findAllByRole("treeitem", { name: "bucket1" });
    expect(buckets.length).toBe(2);

    const [bucket1a, bucket1b] = buckets;
    await userEvent.click(bucket1a);
    expect(bucket1a.ariaSelected).toBe("true");
    expect(bucket1b.ariaSelected).toBe("false");

    expect(bucket1a.ariaExpanded).toBe("true");
    expect(bucket1b.ariaExpanded).toBe("false");

    const folder1 = await screen.findByRole("treeitem", { name: "folder1" });
    expect(folder1).toBeVisible();
  });

  it("keeps a top-level address independent from an identical resource address nested elsewhere", async () => {
    server.use(
      http.get("http://*/v1alpha/filefolder/liststat/:resourceAddress", ({ params }) => {
        const resourceAddress = decodeURI(params.resourceAddress as string);
        if (resourceAddress === `http://test-server/bucket2/`) {
          const body = {
            folderAddress: resourceAddress,
            isRoot: false,
            entries: [],
            subfolderAddresses: [`http://test-server/bucket1/`],
            nextContinuationHandle: null,
          } as ListStatResult;
          return new Response(JSON.stringify(body), { status: 200 });
        }
        return undefined;
      })
    );

    act(() => {
      useAppStore.setState({
        endpoints: new Set(["http://test-endpoint/"]),
        connections: {
          "http://test-endpoint/": testConnection,
        },
      });
    });

    render(component());

    const endpoint = await screen.findByRole("treeitem", { name: "test-endpoint" });
    await userEvent.click(endpoint);

    const bucket2 = await screen.findByRole("treeitem", { name: "bucket2" });
    await userEvent.click(bucket2);

    const buckets = await screen.findAllByRole("treeitem", { name: "bucket1" });
    expect(buckets.length).toBe(2);

    const [topLevelBucket1, nestedBucket1] = buckets;
    await userEvent.click(nestedBucket1);

    expect(nestedBucket1.ariaSelected).toBe("true");
    expect(topLevelBucket1.ariaSelected).toBe("false");

    expect(nestedBucket1.ariaExpanded).toBe("true");
    expect(topLevelBucket1.ariaExpanded).toBe("false");

    const folder1 = await screen.findByRole("treeitem", { name: "folder1" });
    expect(folder1).toBeVisible();
  });

  it("shows context menu with Download and Copy for top-level addresses", async () => {
    act(() => {
      useAppStore.setState({
        endpoints: new Set(["http://test-endpoint/"]),
        connections: {
          "http://test-endpoint/": testConnection,
        },
      });
    });

    render(component());

    const endpoint = await screen.findByRole("treeitem", { name: "test-endpoint" });
    await userEvent.click(endpoint);

    const bucket1 = await screen.findByRole("treeitem", { name: "bucket1" });
    await userEvent.pointer({ keys: "[MouseRight>]", target: bucket1 });

    expect(await screen.findByText("Download")).toBeVisible();
    expect(screen.getByText("Copy")).toBeVisible();
    expect(screen.queryByText("Rename")).not.toBeInTheDocument();
    expect(screen.queryByText("Cut")).not.toBeInTheDocument();
  });

  it("shows full context menu for folders", async () => {
    act(() => {
      useAppStore.setState({
        endpoints: new Set(["http://test-endpoint/"]),
        connections: {
          "http://test-endpoint/": testConnection,
        },
      });
    });

    render(component());

    const endpoint = await screen.findByRole("treeitem", { name: "test-endpoint" });
    await userEvent.click(endpoint);

    const bucket2 = await screen.findByRole("treeitem", { name: "bucket2" });
    await userEvent.click(bucket2);

    const folder3 = await screen.findByRole("treeitem", { name: "folder3" });
    await userEvent.pointer({ keys: "[MouseRight>]", target: folder3 });

    expect(await screen.findByText("Download")).toBeVisible();
    expect(screen.getByText("Copy")).toBeVisible();
    expect(screen.getByText("Rename")).toBeVisible();
    expect(screen.getByText("Cut")).toBeVisible();
    expect(screen.getByText("Delete")).toBeVisible();
  });

  it.each(["folder3", "bucket1"])("refreshes a renamed folder while %s is selected", async (selectedName) => {
    const parent = "http://test-server/bucket2/";
    const oldAddress = `${parent}folder3/`;
    const newAddress = `${parent}renamed-folder/`;
    const folders = new Set([oldAddress]);
    const deleted = vi.fn();

    server.use(
      http.get("http://*/v1alpha/filefolder/liststat/:resourceAddress", ({ params }) => {
        const resourceAddress = params.resourceAddress as string;
        if (
          resourceAddress === parent ||
          resourceAddress.startsWith(oldAddress) ||
          resourceAddress.startsWith(newAddress)
        ) {
          return HttpResponse.json({
            folderAddress: resourceAddress,
            isRoot: false,
            entries: [],
            subfolderAddresses: resourceAddress === parent ? Array.from(folders) : [],
            nextContinuationHandle: null,
          });
        }
        return undefined;
      }),
      http.put("http://*/v1alpha/filefolder/:resourceAddress", ({ params }) => {
        expect(params.resourceAddress).toBe(newAddress.slice(0, -1));
        folders.add(newAddress);
        return new HttpResponse(null, { status: 204 });
      }),
      http.delete("http://*/v1alpha/filefolder/list/:resourceAddress", ({ params }) => {
        expect(params.resourceAddress).toBe(oldAddress);
        folders.delete(oldAddress);
        deleted();
        return new HttpResponse(null, { status: 204 });
      })
    );

    act(() => {
      useAppStore.setState({
        endpoints: new Set(["http://test-endpoint/"]),
        connections: {
          "http://test-endpoint/": { ...testConnection, endpoint: "http://test-endpoint/", currentFolder: oldAddress },
        },
      });
    });

    render(component());
    await userEvent.click(await screen.findByRole("treeitem", { name: "test-endpoint" }));
    await userEvent.click(await screen.findByRole("treeitem", { name: "bucket2" }));
    const folder = await screen.findByRole("treeitem", { name: "folder3" });
    await userEvent.click(await screen.findByRole("treeitem", { name: selectedName }));

    await userEvent.pointer({ keys: "[MouseRight]", target: folder });
    await userEvent.click(await screen.findByText("Rename"));
    const input = await screen.findByDisplayValue("folder3");
    await userEvent.clear(input);
    await userEvent.type(input, "renamed-folder");
    await userEvent.click(screen.getByRole("button", { name: "Rename" }));

    await waitFor(() => expect(deleted).toHaveBeenCalled());
    expect(await screen.findByRole("treeitem", { name: "renamed-folder" })).toBeVisible();
    await waitFor(() => expect(screen.queryByRole("treeitem", { name: "folder3" })).not.toBeInTheDocument());
  });

  it("updates an unselected tree parent in response to directory events", async () => {
    const connection = {
      ...testConnection,
      endpoint: "http://test-endpoint/",
      currentFolder: "http://test-server/bucket2/",
    };
    let handleEvent: (event: OmniEvent) => void = vi.fn();
    const subscription = {
      onEvent: (handler: (event: OmniEvent) => void) => {
        handleEvent = handler;
        return vi.fn();
      },
    };
    const DirectoryEvents = () => {
      useStorageSubscription(connection, subscription);
      return null;
    };

    act(() => {
      useAppStore.setState({
        endpoints: new Set([connection.endpoint]),
        connections: { [connection.endpoint]: connection },
      });
    });

    render(component(<DirectoryEvents />));
    await userEvent.click(await screen.findByRole("treeitem", { name: "test-endpoint" }));
    await userEvent.click(await screen.findByRole("treeitem", { name: "bucket2" }));
    await userEvent.click(await screen.findByRole("treeitem", { name: "folder3" }));

    const event = (eventType: "omni.storage.dir_created" | "omni.storage.dir_deleted", name: string): OmniEvent => ({
      eventType,
      principalIdentity: "test-user",
      occurredAt: "2026-10-02T00:00:00Z",
      publishedAt: "2026-10-02T00:00:00Z",
      message: { resourceAddress: `${connection.currentFolder}${name}/`, resourceIdentity: name },
    });

    act(() => handleEvent(event("omni.storage.dir_created", "renamed-folder")));
    expect(await screen.findByRole("treeitem", { name: "renamed-folder" })).toBeVisible();

    act(() => handleEvent(event("omni.storage.dir_deleted", "folder3")));
    await waitFor(() => expect(screen.queryByRole("treeitem", { name: "folder3" })).not.toBeInTheDocument());
    expect(screen.getByRole("treeitem", { name: "renamed-folder" })).toBeVisible();
  });

  it("highlights the tree node when its context menu is open", async () => {
    act(() => {
      useAppStore.setState({
        endpoints: new Set(["http://test-endpoint/"]),
        connections: {
          "http://test-endpoint/": testConnection,
        },
      });
    });

    render(component());

    const endpoint = await screen.findByRole("treeitem", { name: "test-endpoint" });
    await userEvent.click(endpoint);

    const bucket1 = await screen.findByRole("treeitem", { name: "bucket1" });
    const bucket2 = await screen.findByRole("treeitem", { name: "bucket2" });

    await userEvent.pointer({ keys: "[MouseRight>]", target: bucket1 });
    expect(await screen.findByText("Download")).toBeVisible();
    expect(bucket1).toHaveAttribute("data-context-menu-active");
    expect(bucket2).not.toHaveAttribute("data-context-menu-active");

    await userEvent.pointer({ keys: "[MouseRight>]", target: bucket2 });
    expect(await screen.findByText("Download")).toBeVisible();
    expect(bucket1).not.toHaveAttribute("data-context-menu-active");
    expect(bucket2).toHaveAttribute("data-context-menu-active");
  });

  describe("default-server mode", () => {
    it("hides the Add Connection node when defaultDiscovery is configured", async () => {
      mocks.defaultDiscovery = "https://stack.example/";
      render(component());

      await screen.findByRole("treeitem", { name: "Omniverse Storage" });
      expect(screen.queryByRole("treeitem", { name: "Add Connection" })).toBeNull();
    });

    it("does not show the delete context menu on a connection in default mode", async () => {
      mocks.defaultDiscovery = "https://stack.example/";
      act(() => {
        useAppStore.setState({
          endpoints: new Set(["http://test-endpoint/"]),
          connections: {
            "http://test-endpoint/": testConnection,
          },
        });
      });

      render(component());

      const endpoint = await screen.findByRole("treeitem", { name: "test-endpoint" });
      await userEvent.pointer({ keys: "[MouseRight>]", target: endpoint });

      expect(screen.queryByText("Delete")).toBeNull();
      expect(screen.queryByRole("treeitem", { name: "test-endpoint" })).toBeVisible();
    });

    it("logs the user out of an authenticated connection without a delete option", async () => {
      mocks.defaultDiscovery = "https://stack.example/";
      act(() => {
        useAppStore.setState({
          endpoints: new Set(["http://test-endpoint/"]),
          connections: {
            "http://test-endpoint/": {
              ...testConnection,
              endpoint: "http://test-endpoint/",
              info: { discovery: "http://discovery", auth: createTestAuth() },
            },
          },
        });
      });

      render(component());

      const endpoint = await screen.findByRole("treeitem", { name: /test-endpoint/ });
      await userEvent.pointer({ keys: "[MouseRight>]", target: endpoint });

      const logoutMenuItem = await screen.findByText("Log Out");
      expect(screen.queryByText("Delete")).toBeNull();
      await userEvent.click(logoutMenuItem);

      await waitFor(() => expect(mocks.logout).toHaveBeenCalledWith(mocks.authConfig, "test-id-token"));
      expect(useAppStore.getState().connections["http://test-endpoint/"].info.auth).toBeNull();
    });
  });

  it("offers both delete and logout for an authenticated connection", async () => {
    act(() => {
      useAppStore.setState({
        endpoints: new Set(["http://test-endpoint/"]),
        connections: {
          "http://test-endpoint/": {
            ...testConnection,
            endpoint: "http://test-endpoint/",
            info: { discovery: "http://discovery", auth: createTestAuth() },
          },
        },
      });
    });

    render(component());

    const endpoint = await screen.findByRole("treeitem", { name: /test-endpoint/ });
    await userEvent.pointer({ keys: "[MouseRight>]", target: endpoint });

    expect(await screen.findByText("Delete")).toBeVisible();
    expect(screen.getByText("Log Out")).toBeVisible();
  });

  it("displays an error icon on a tree node when loading fails", async () => {
    server.use(
      http.get("http://*/v1alpha/filefolder/liststat/:resourceAddress", () => {
        return new Response("Test error.", { status: 500 });
      })
    );

    act(() => {
      useAppStore.setState({
        endpoints: new Set(["http://test-endpoint/"]),
        connections: {
          "http://test-endpoint/": testConnection,
        },
      });
    });

    render(component());

    const endpoint = await screen.findByRole("treeitem", { name: "test-endpoint" });
    await userEvent.click(endpoint);

    const bucket1 = await screen.findByRole("treeitem", { name: "bucket1" });
    await userEvent.click(bucket1);

    await waitFor(() => {
      expect(bucket1.querySelector(".tabler-icon-exclamation-circle")).toBeInTheDocument();
    });
  });
});
