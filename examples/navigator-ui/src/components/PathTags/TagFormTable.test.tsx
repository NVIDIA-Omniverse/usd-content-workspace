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

import { describe, expect, it, vi } from "vitest";
import { PathShortInfo, PathType } from "@/state/path";
import { Connection } from "@/state/slices/connectionSlice";
import { render, screen, userEvent, waitFor, within } from "@/test-utils";
import { TagFormTable } from "./TagFormTable";

vi.mock("@/hooks/useSetTagsMutation", () => ({
  useSetTagsMutation: () => ({
    mutate: vi.fn(),
    isPending: false,
    error: null,
  }),
}));

const mockConnection: Connection = {
  endpoint: "http://test-server",
  info: {
    discovery: "http://test-server",
    auth: null,
  },
  currentFolder: "http://test-server/bucket1/",
  selectedPaths: new Set(),
};

const mockPath: PathShortInfo = {
  type: PathType.File,
  resourceAddress: "http://test-server/bucket1/test-file.txt",
};

describe("TagFormTable", () => {
  it("Submit disabled when form is pristine and enables after change", async () => {
    render(
      <TagFormTable
        connection={mockConnection}
        path={mockPath}
        tags={{ existing: "tag" }}
        etag="etag-123"
        onReset={vi.fn()}
      />
    );

    const submitButton = screen.getByRole("button", { name: "Submit" });
    expect(submitButton).toBeDisabled();

    const keyInput = screen.getByDisplayValue("existing");
    await userEvent.type(keyInput, "_updated");

    await userEvent.tab();

    await waitFor(() => {
      expect(submitButton).toBeEnabled();
    });
  });

  it("disables submit when validation error exists", async () => {
    render(
      <TagFormTable
        connection={mockConnection}
        path={mockPath}
        tags={{ duplicate: "a", other: "b" }}
        etag="etag-123"
        onReset={vi.fn()}
      />
    );

    const submitButton = screen.getByRole("button", { name: "Submit" });

    // Add duplicate key
    const addRow = screen.getAllByRole("row").at(-1)!;
    const keyInput = within(addRow).getByPlaceholderText("Key");
    const valueInput = screen.getByPlaceholderText("Value (optional)");
    await userEvent.type(keyInput, "duplicate");
    await userEvent.type(valueInput, "val");

    const addButton = screen.getByTitle("Add tag");
    await userEvent.click(addButton);

    expect(submitButton).toBeDisabled();
  });

  it("enables submit when only a tag value is changed", async () => {
    render(
      <TagFormTable
        connection={mockConnection}
        path={mockPath}
        tags={{ key: "value" }}
        etag="etag-1"
        onReset={vi.fn()}
      />
    );

    const submitButton = screen.getByRole("button", { name: "Submit" });
    expect(submitButton).toBeDisabled();

    const valueInput = screen.getByDisplayValue("value");
    await userEvent.type(valueInput, "_updated");

    await waitFor(() => expect(submitButton).toBeEnabled());
  });

  it("removes a tag when the delete button is clicked", async () => {
    render(
      <TagFormTable
        connection={mockConnection}
        path={mockPath}
        tags={{ remove: "me", keep: "yes" }}
        etag="etag-123"
        onReset={vi.fn()}
      />
    );

    expect(screen.getByDisplayValue("remove")).toBeVisible();

    const deleteButton = screen.getByTitle("Remove remove");
    await userEvent.click(deleteButton);

    await waitFor(() => {
      expect(screen.queryByDisplayValue("remove")).not.toBeInTheDocument();
    });

    const submitButton = screen.getByRole("button", { name: "Submit" });
    expect(submitButton).toBeEnabled();
  });

  it("adds a new tag when key and value are valid", async () => {
    render(<TagFormTable connection={mockConnection} path={mockPath} tags={{}} etag="etag-456" onReset={vi.fn()} />);

    const addRow = screen.getAllByRole("row").at(-1)!;
    const keyInput = within(addRow).getByPlaceholderText("Key");
    const valueInput = within(addRow).getByPlaceholderText("Value (optional)");

    await userEvent.type(keyInput, "newtag");
    await userEvent.type(valueInput, "newvalue");

    const addButton = screen.getByTitle("Add tag");
    await userEvent.click(addButton);

    await waitFor(() => {
      expect(screen.getByDisplayValue("newtag")).toBeVisible();
    });

    const updatedAddRow = screen.getAllByRole("row").at(-1)!;
    expect(within(updatedAddRow).getByPlaceholderText("Key")).toHaveValue("");
    expect(within(updatedAddRow).getByPlaceholderText("Value (optional)")).toHaveValue("");
  });

  it("does not add a tag when the key is invalid", async () => {
    render(<TagFormTable connection={mockConnection} path={mockPath} tags={{}} etag="etag-789" onReset={vi.fn()} />);

    const addRow = screen.getAllByRole("row").at(-1)!;
    const keyInput = within(addRow).getByPlaceholderText("Key");
    const valueInput = within(addRow).getByPlaceholderText("Value (optional)");

    await userEvent.type(keyInput, "invalid-key");
    await userEvent.type(valueInput, "value");

    const addButton = screen.getByTitle("Add tag");
    const rowCountBefore = screen.getAllByRole("row").length;

    await userEvent.click(addButton);

    await waitFor(() => {
      expect(screen.getAllByRole("row").length).toBe(rowCountBefore);
      expect(
        screen.getByText("Key contains invalid characters. Only letters, numbers, underscores, and spaces are allowed.")
      ).toBeVisible();
      // The key input in the add row should still contain the invalid key value
      expect(within(addRow).getByPlaceholderText("Key")).toHaveValue("invalid-key");
    });
  });

  it("does not add a tag when the value is invalid", async () => {
    render(
      <TagFormTable connection={mockConnection} path={mockPath} tags={{}} etag="etag-invalid-value" onReset={vi.fn()} />
    );

    const addRow = screen.getAllByRole("row").at(-1)!;
    const keyInput = within(addRow).getByPlaceholderText("Key");
    const valueInput = within(addRow).getByPlaceholderText("Value (optional)");

    await userEvent.type(keyInput, "validkey");
    await userEvent.type(valueInput, "invalid value!");

    const rowCountBefore = screen.getAllByRole("row").length;

    const addButton = screen.getByTitle("Add tag");
    await userEvent.click(addButton);

    await waitFor(() => {
      // Number of rows should remain unchanged because the tag was not added
      expect(screen.getAllByRole("row").length).toBe(rowCountBefore);
      expect(
        screen.getByText(
          "Value contains invalid characters. Only letters, numbers, underscores, and spaces are allowed."
        )
      ).toBeVisible();
      expect(within(addRow).getByPlaceholderText("Value (optional)")).toHaveValue("invalid value!");
    });
  });
});
