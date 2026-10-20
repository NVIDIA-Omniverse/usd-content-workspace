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
import { screen, waitFor, within } from "@testing-library/react";
import { ContextMenuProvider } from "mantine-contextmenu";
import { http } from "msw";
import { setupServer } from "msw/node";
import { afterEach, beforeEach } from "vitest";
import { Notifications } from "@mantine/notifications";
import { FileVersions } from "@/components/FileVersions/FileVersions";
import { VersionPage } from "@/services/storage/versioningService";
import { createFileListItem } from "@/state/path";
import { render, userEvent } from "@/test-utils";
import { mockGetBoundingClientRect } from "@/test-utils/element";
import { testConnection, testEndpoints } from "@/test-utils/msw";
import { createTestQueryClient } from "@/test-utils/utils";
import { formatDate } from "@/util/date";

describe("FileVersions", () => {
  const file = createFileListItem({
    resourceAddress: "https://test-endpoint/test.dat",
    resourceIdentity: "https://test-endpoint/test.dat?version=3",
    metadata: {
      lastModifiedTimestamp: "2024-05-15T16:00:00Z",
      dataObjectSize: 0,
    },
  });

  const server = setupServer(...testEndpoints);

  beforeEach(() => {
    server.listen();
    mockGetBoundingClientRect((element) => {
      return element.getAttribute("aria-label") === "Versions";
    });
  });

  afterEach(() => {
    server.restoreHandlers();
    server.close();
  });

  const component = () => {
    return (
      <QueryClientProvider client={createTestQueryClient()}>
        <ContextMenuProvider>
          <FileVersions file={file} connection={testConnection} />
          <Notifications withinPortal={false} />
        </ContextMenuProvider>
      </QueryClientProvider>
    );
  };

  it("displays versions", async () => {
    render(component());

    const version1 = await screen.findByLabelText(formatDate("2024-05-15T14:00:00Z"));
    expect(version1).toBeVisible();

    const version1Size = await within(version1).findByText("1 Bytes");
    expect(version1Size).toBeVisible();

    const version2 = await screen.findByLabelText(formatDate("2024-05-15T15:00:00Z"));
    expect(version2).toBeVisible();

    const version2Size = await within(version2).findByText("2 Bytes");
    expect(version2Size).toBeVisible();

    const version3 = await screen.findByLabelText(formatDate("2024-05-15T16:00:00Z"));
    expect(version3).toBeVisible();

    const version3Size = await within(version3).findByText("3 Bytes");
    expect(version3Size).toBeVisible();

    const versions = within(screen.getByLabelText("Versions")).getAllByLabelText(/\d{2}\/\d{2}\/\d{4}/);
    expect(versions.map((version) => version.getAttribute("aria-label"))).toEqual([
      formatDate("2024-05-15T16:00:00Z"),
      formatDate("2024-05-15T15:00:00Z"),
      formatDate("2024-05-15T14:00:00Z"),
    ]);
  });

  it("displays versions from multiple pages", async () => {
    server.use(
      http.get("http://test-endpoint/v1alpha/versioning/*/versions", ({ request }) => {
        const url = new URL(request.url);
        const continuationHandle = url.searchParams.get("continuation_handle") ?? "";

        let body: VersionPage;
        switch (continuationHandle) {
          case "page2":
            body = {
              items: [
                {
                  resourceInfo: {
                    resourceIdentity: "2.dat",
                    metadata: {
                      dataObjectSize: 2,
                      lastModifiedTimestamp: "2024-05-15T15:00:00Z",
                    },
                  },
                },
              ],
              nextContinuationHandle: null,
              versionsOrder: "oldest_first",
            };
            break;
          default:
            body = {
              items: [
                {
                  resourceInfo: {
                    resourceIdentity: "1.dat",
                    metadata: {
                      dataObjectSize: 1,
                      lastModifiedTimestamp: "2024-05-15T14:00:00Z",
                    },
                  },
                },
              ],
              nextContinuationHandle: "page2",
              versionsOrder: "oldest_first",
            };
            break;
        }
        return new Response(JSON.stringify(body));
      })
    );

    render(component());

    const version1 = await screen.findByLabelText(formatDate("2024-05-15T14:00:00Z"));
    expect(version1).toBeVisible();

    const version1Size = await within(version1).findByText("1 Bytes");
    expect(version1Size).toBeVisible();

    const version2 = await screen.findByLabelText(formatDate("2024-05-15T15:00:00Z"));
    expect(version2).toBeVisible();

    const version2Size = await within(version2).findByText("2 Bytes");
    expect(version2Size).toBeVisible();

    const versions = within(screen.getByLabelText("Versions")).getAllByLabelText(/\d{2}\/\d{2}\/\d{4}/);
    expect(versions.map((version) => version.getAttribute("aria-label"))).toEqual([
      formatDate("2024-05-15T15:00:00Z"),
      formatDate("2024-05-15T14:00:00Z"),
    ]);
  });

  it("keeps loading until every version page has been fetched", async () => {
    let resolvePage2: () => void = () => undefined;
    const page2Ready = new Promise<void>((resolve) => {
      resolvePage2 = resolve;
    });
    let page2Requested = false;

    server.use(
      http.get("http://test-endpoint/v1alpha/versioning/*/versions", async ({ request }) => {
        const url = new URL(request.url);
        const continuationHandle = url.searchParams.get("continuation_handle") ?? "";

        if (continuationHandle === "page2") {
          page2Requested = true;
          await page2Ready;
          return new Response(
            JSON.stringify({
              items: [
                {
                  resourceInfo: {
                    resourceIdentity: "2.dat",
                    metadata: {
                      dataObjectSize: 2,
                      lastModifiedTimestamp: "2024-05-15T15:00:00Z",
                    },
                  },
                },
              ],
              nextContinuationHandle: null,
              versionsOrder: "oldest_first",
            } satisfies VersionPage)
          );
        }

        return new Response(
          JSON.stringify({
            items: [
              {
                resourceInfo: {
                  resourceIdentity: "1.dat",
                  metadata: {
                    dataObjectSize: 1,
                    lastModifiedTimestamp: "2024-05-15T14:00:00Z",
                  },
                },
              },
            ],
            nextContinuationHandle: "page2",
            versionsOrder: "oldest_first",
          } satisfies VersionPage)
        );
      })
    );

    render(component());

    try {
      await waitFor(() => expect(page2Requested).toBe(true));

      expect(screen.getByRole("progressbar", { name: "Loading versions" })).toBeVisible();
      expect(screen.queryByLabelText(formatDate("2024-05-15T14:00:00Z"))).not.toBeInTheDocument();

      resolvePage2();

      const version2 = await screen.findByLabelText(formatDate("2024-05-15T15:00:00Z"));
      expect(version2).toBeVisible();

      await waitFor(() => {
        expect(screen.queryByRole("progressbar", { name: "Loading versions" })).not.toBeInTheDocument();
      });
    } finally {
      resolvePage2();
    }
  });

  it("displays newest-first versions after every page has been fetched", async () => {
    let resolvePage2: () => void = () => undefined;
    const page2Ready = new Promise<void>((resolve) => {
      resolvePage2 = resolve;
    });
    let page2Requested = false;

    server.use(
      http.get("http://test-endpoint/v1alpha/versioning/*/versions", async ({ request }) => {
        const url = new URL(request.url);
        const continuationHandle = url.searchParams.get("continuation_handle") ?? "";

        if (continuationHandle === "page2") {
          page2Requested = true;
          await page2Ready;
          return new Response(
            JSON.stringify({
              items: [
                {
                  resourceInfo: {
                    resourceIdentity: "1.dat",
                    metadata: {
                      dataObjectSize: 1,
                      lastModifiedTimestamp: "2024-05-15T14:00:00Z",
                    },
                  },
                },
              ],
              nextContinuationHandle: null,
              versionsOrder: "newest_first",
            } satisfies VersionPage)
          );
        }

        return new Response(
          JSON.stringify({
            items: [
              {
                resourceInfo: {
                  resourceIdentity: "2.dat",
                  metadata: {
                    dataObjectSize: 2,
                    lastModifiedTimestamp: "2024-05-15T15:00:00Z",
                  },
                },
              },
            ],
            next_continuation_handle: "page2",
            versions_order: "newest_first",
          })
        );
      })
    );

    render(component());

    try {
      await waitFor(() => expect(page2Requested).toBe(true));

      expect(screen.getByRole("progressbar", { name: "Loading versions" })).toBeVisible();
      expect(screen.queryByLabelText(formatDate("2024-05-15T15:00:00Z"))).not.toBeInTheDocument();
      expect(screen.queryByLabelText(formatDate("2024-05-15T14:00:00Z"))).not.toBeInTheDocument();

      resolvePage2();

      const firstPageVersion = await screen.findByLabelText(formatDate("2024-05-15T15:00:00Z"));
      expect(firstPageVersion).toBeVisible();

      const secondPageVersion = await screen.findByLabelText(formatDate("2024-05-15T14:00:00Z"));
      expect(secondPageVersion).toBeVisible();

      await waitFor(() => {
        expect(screen.queryByRole("progressbar", { name: "Loading versions" })).not.toBeInTheDocument();
      });

      const versions = within(screen.getByLabelText("Versions")).getAllByLabelText(/\d{2}\/\d{2}\/\d{4}/);
      expect(versions.map((version) => version.getAttribute("aria-label"))).toEqual([
        formatDate("2024-05-15T15:00:00Z"),
        formatDate("2024-05-15T14:00:00Z"),
      ]);
    } finally {
      resolvePage2();
    }
  });

  test("restores a version", async () => {
    server.use(
      http.post("http://test-endpoint/v1alpha/fileobject/by-identity/*/copy", () => {
        return new Response(JSON.stringify({ resourceIdentity: "restored.dat" }), {
          status: 200,
          headers: { "Content-Type": "application/json" },
        });
      })
    );

    render(component());

    const version = await screen.findByLabelText(formatDate("2024-05-15T15:00:00Z"));
    await userEvent.pointer({ keys: "[MouseRight>]", target: version });

    const restore = await screen.findByText("Restore");
    expect(restore).toBeVisible();
    await userEvent.click(restore);

    const notifications = await screen.findAllByText("Version restored successfully.");
    expect(notifications[0]).toBeVisible();
  });

  test("displays an error notification if restore fails", async () => {
    server.use(
      http.post("http://test-endpoint/v1alpha/fileobject/by-identity/*/copy", () => {
        return new Response("Test error.", { status: 500 });
      })
    );

    render(component());

    const version = await screen.findByLabelText(formatDate("2024-05-15T14:00:00Z"));
    await userEvent.pointer({ keys: "[MouseRight>]", target: version });

    const restore = await screen.findByText("Restore");
    await userEvent.click(restore);

    const notifications = await screen.findAllByText("Version restore failed.");
    expect(notifications[0]).toBeVisible();

    const error = await screen.findByText(/HTTPError: Request failed with status code 500/);
    expect(error).toBeVisible();
  });
});
