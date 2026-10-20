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

import { http, HttpResponse } from "msw";
import { setupServer } from "msw/node";
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { notifications } from "@mantine/notifications";
import { PathTags } from "@/components/PathTags/PathTags";
import { Path, PathType } from "@/state/path";
import { Connection } from "@/state/slices/connectionSlice";
import { render, screen, userEvent, waitFor, within } from "@/test-utils";

vi.mock("@mantine/notifications", () => ({
  notifications: {
    show: vi.fn(),
    hide: vi.fn(),
    update: vi.fn(),
    clean: vi.fn(),
    cleanQueue: vi.fn(),
  } as unknown as typeof notifications,
}));

let forceMutationError = false;

vi.mock("@/hooks/useSetTagsMutation", () => ({
  useSetTagsMutation: () => ({
    mutate: (_args: unknown, callbacks: { onSuccess?: () => void; onError?: () => void }) => {
      if (forceMutationError) {
        callbacks.onError?.();
      } else {
        callbacks.onSuccess?.();
      }
    },
    isPending: false,
    error: null,
  }),
}));

const mockPath: Path = {
  name: "test-file.txt",
  type: PathType.File,
  displayedType: "txt",
  isFolderLike: false,
  size: "1.00 KB",
  bytes: 1024,
  modified: "02/27/2024 12:00:00",
  resourceAddress: "http://test-server/bucket1/test-file.txt",
  resourceIdentity: "1",
  metadata: {
    dataObjectSize: 1024,
    lastModifiedTimestamp: new Date().toISOString(),
  },
};

const mockConnection: Connection = {
  endpoint: "http://test-server",
  info: {
    discovery: "http://test-server",
    auth: null,
  },
  currentFolder: "http://test-server/bucket1/",
  selectedPaths: new Set(),
};

const handlers = [
  http.get("http://test-server/api/v2/services", () => new HttpResponse(null, { status: 404 })),
  http.get("http://test-server/api/v1/services", () =>
    HttpResponse.json({
      services: [{ id: "storage", name: "Storage Service", type: "storage", rest: "http://test-server" }],
    })
  ),
  http.post("http://test-server/v1alpha/metadata/*", () => {
    return HttpResponse.json({
      acl: {
        value: ["read", "write"],
        etag: "",
      },
      tags: {
        value: {
          test_key: "test_value",
          another_key: "another_value",
        },
        etag: "test-etag-123",
      },
      thumbnails: [],
    });
  }),

  http.put("http://test-server/v1alpha/metadata/*/tags", () => {
    return HttpResponse.json({});
  }),
];

const server = setupServer(...handlers);

beforeAll(() => {
  server.listen();
});

afterAll(() => {
  server.close();
});

afterEach(() => {
  server.resetHandlers();
  vi.clearAllMocks();
  forceMutationError = false;
});

describe("TagForm", () => {
  it("renders tags list correctly", async () => {
    render(<PathTags path={mockPath} connection={mockConnection} />);
    await waitFor(() => {
      expect(screen.getByDisplayValue("test_key")).toBeVisible();
      expect(screen.getByDisplayValue("test_value")).toBeVisible();
      expect(screen.getByDisplayValue("another_key")).toBeVisible();
      expect(screen.getByDisplayValue("another_value")).toBeVisible();
    });
  });

  it("shows error state when API fails", async () => {
    server.use(
      http.post("http://test-server/v1alpha/metadata/*", () => {
        return new HttpResponse(JSON.stringify({ error: "Failed to load tags" }), {
          status: 500,
          headers: { "Content-Type": "application/json" },
        });
      })
    );

    render(<PathTags path={mockPath} connection={mockConnection} />);

    await waitFor(() => {
      expect(screen.getByPlaceholderText("Key")).toBeVisible();
      expect(screen.getByPlaceholderText("Value (optional)")).toBeVisible();
    });
  });

  it("submits changes and shows success notification", async () => {
    render(<PathTags path={mockPath} connection={mockConnection} />);

    await waitFor(() => {
      expect(screen.getAllByLabelText("Key")[0]).toBeVisible();
    });

    const keyInput = screen.getAllByDisplayValue("test_key")[0];
    await userEvent.type(keyInput, "_updated");

    const submitButton = screen.getByRole("button", { name: "Submit" });

    await waitFor(() => {
      expect(submitButton).toBeEnabled();
    });

    await userEvent.click(submitButton);

    await waitFor(() => {
      expect(notifications.show).toHaveBeenCalledWith(
        expect.objectContaining({
          title: "Success",
          message: "Tags updated successfully.",
          color: "green",
        })
      );
    });
  });

  it("shows empty state when no tags exist", async () => {
    server.use(
      http.post("http://test-server/v1alpha/metadata/*", () => {
        return HttpResponse.json({
          acl: {
            value: ["read", "write"],
            etag: "",
          },
          tags: {
            value: {},
            etag: "",
          },
          thumbnails: [],
        });
      })
    );

    render(<PathTags path={mockPath} connection={mockConnection} />);

    await waitFor(() => {
      expect(screen.getByPlaceholderText("Key")).toBeVisible();
      expect(screen.getByPlaceholderText("Value (optional)")).toBeVisible();
    });
  });

  it("validates required key field", async () => {
    render(<PathTags path={mockPath} connection={mockConnection} />);

    await waitFor(() => {
      expect(screen.getAllByLabelText("Key")[0]).toBeVisible();
    });

    const valueInput = screen.getByPlaceholderText("Value (optional)");
    await userEvent.type(valueInput, "test_value");

    const addButton = screen.getByTitle("Add tag");
    await userEvent.click(addButton);

    await waitFor(() => {
      expect(screen.getByText("Key cannot be empty")).toBeVisible();
    });
  });

  it("prevents duplicate keys", async () => {
    render(<PathTags path={mockPath} connection={mockConnection} />);

    await waitFor(() => {
      expect(screen.getByDisplayValue("test_key")).toBeVisible();
    });

    const rows = screen.getAllByRole("row");
    const addRow = rows.at(-1)!;
    const keyInput = within(addRow).getByPlaceholderText("Key");
    const valueInput = within(addRow).getByPlaceholderText("Value (optional)");

    await userEvent.type(keyInput, "test_key");
    await userEvent.type(valueInput, "duplicate_value");

    const addButton = screen.getByTitle("Add tag");
    await userEvent.click(addButton);

    await waitFor(() => {
      expect(screen.getByText("Key must be unique")).toBeVisible();
    });
  });

  it("handles API errors when setting tags", async () => {
    forceMutationError = true;

    render(<PathTags path={mockPath} connection={mockConnection} />);

    await waitFor(() => {
      expect(screen.getAllByLabelText("Key")[0]).toBeVisible();
    });

    const keyInput = screen.getByDisplayValue("test_key");
    await userEvent.type(keyInput, "_modified");

    const submitButton = screen.getByRole("button", { name: "Submit" });
    await userEvent.click(submitButton);

    await waitFor(() => {
      expect(notifications.show).toHaveBeenCalledWith(
        expect.objectContaining({
          title: "Failed to submit tag changes",
          message: "Please try again.",
          color: "red",
        })
      );
    });
  });
});
