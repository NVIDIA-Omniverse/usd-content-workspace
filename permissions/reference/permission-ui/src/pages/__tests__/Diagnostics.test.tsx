/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import { Diagnostics } from "../Diagnostics";
import { render } from "../../util/tests/render";
import { mockConfigWithUserService } from "../../util/tests/config";
import { mockDiagnosticsBatchResponse, server } from "../../util/tests/msw";

const BATCH_URL = "http://localhost:8080/v1beta/diagnostics/authorize/batch/";

function renderDiagnostics() {
  return render({
    initialEntries: ["/diagnostics"],
    children: <Diagnostics />,
  });
}

async function fillRequiredFields(user: ReturnType<typeof userEvent.setup>) {
  await user.type(
    screen.getByRole("combobox", { name: "Service" }),
    "document-service",
  );
  await user.type(screen.getByRole("combobox", { name: "Action" }), "read");
}

describe("Diagnostics", () => {
  it("locks the principal type selector to User", () => {
    renderDiagnostics();

    expect(screen.getByLabelText("Principal type: User")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Principal type: User" }),
    ).not.toBeInTheDocument();
  });

  it("renders the form with all fields empty", () => {
    renderDiagnostics();

    expect(screen.getByText("Diagnostics")).toBeVisible();

    expect(screen.getByRole("textbox", { name: "Principal" })).toHaveValue("");
    expect(screen.getByRole("combobox", { name: "Service" })).toHaveValue("");
    expect(screen.getByRole("combobox", { name: "Action" })).toHaveValue("");
    expect(screen.getByRole("textbox", { name: "Context" })).toHaveValue("");
    expect(
      screen.getByRole("combobox", { name: "Resource type" }),
    ).toHaveValue("");
    expect(screen.getByRole("textbox", { name: "Resource ID" })).toHaveValue(
      "",
    );

    expect(
      screen.getByRole("button", { name: "Run diagnostics" }),
    ).toBeVisible();
    expect(screen.getByRole("button", { name: "Reset" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Add resource" })).toBeVisible();
  });

  it("validates service and action are required on empty submit", async () => {
    const user = userEvent.setup();
    renderDiagnostics();

    await user.click(screen.getByRole("button", { name: "Run diagnostics" }));

    expect(await screen.findByText("Service is required.")).toBeVisible();
    expect(screen.getByText("Action is required.")).toBeVisible();
  });

  it("validates resource type is required when resource ID is specified", async () => {
    const user = userEvent.setup();
    renderDiagnostics();

    await fillRequiredFields(user);
    await user.type(
      screen.getByRole("textbox", { name: "Resource ID" }),
      "doc-123",
    );

    await user.click(screen.getByRole("button", { name: "Run diagnostics" }));

    expect(
      await screen.findByText("Resource type is required."),
    ).toBeVisible();
  });

  it("validates context must be valid JSON", async () => {
    const user = userEvent.setup();
    renderDiagnostics();

    await fillRequiredFields(user);

    const contextInput = screen.getByRole("textbox", { name: "Context" });
    fireEvent.change(contextInput, { target: { value: "not valid json" } });
    fireEvent.blur(contextInput);

    await user.click(screen.getByRole("button", { name: "Run diagnostics" }));

    expect(
      await screen.findByText("Context must be valid JSON."),
    ).toBeVisible();
  });

  it("adds and removes resource rows", async () => {
    const user = userEvent.setup();
    renderDiagnostics();

    expect(screen.getAllByPlaceholderText("id")).toHaveLength(1);

    await user.click(screen.getByRole("button", { name: "Add resource" }));
    expect(screen.getAllByPlaceholderText("id")).toHaveLength(2);

    await user.click(
      screen.getByRole("button", { name: "Remove resource 2" }),
    );
    expect(screen.getAllByPlaceholderText("id")).toHaveLength(1);
  });

  it("expands a resource into its hierarchy with empty types", async () => {
    const user = userEvent.setup();
    renderDiagnostics();

    await user.type(
      screen.getByRole("textbox", { name: "Resource ID" }),
      "https://host.example.com/a/b/c",
    );

    await user.click(
      screen.getByRole("button", {
        name: "Expand resource 1 into a hierarchy",
      }),
    );

    const idInputs = screen.getAllByPlaceholderText("id");
    expect(idInputs).toHaveLength(4);
    expect(idInputs[0]).toHaveValue("https://host.example.com/a/b/c");
    expect(idInputs[1]).toHaveValue("https://host.example.com/a/b/");
    expect(idInputs[2]).toHaveValue("https://host.example.com/a/");
    expect(idInputs[3]).toHaveValue("https://host.example.com/");

    const typeInputs = screen.getAllByPlaceholderText("type");
    expect(typeInputs[1]).toHaveValue("");
    expect(typeInputs[2]).toHaveValue("");
    expect(typeInputs[3]).toHaveValue("");

    await user.click(
      screen.getByRole("button", {
        name: "Expand resource 1 into a hierarchy",
      }),
    );

    expect(screen.getAllByPlaceholderText("id")).toHaveLength(4);
  });

  it("expands a hierarchy reusing the same type when parent_type is unset", async () => {
    const user = userEvent.setup();
    renderDiagnostics();

    await user.type(
      screen.getByRole("combobox", { name: "Service" }),
      "document-service",
    );

    const typeInput = screen.getByRole("combobox", { name: "Resource type" });
    await user.click(typeInput);
    expect(await screen.findByRole("option", { name: "Folder" })).toBeVisible();
    await user.type(typeInput, "Folder");

    await user.type(
      screen.getByRole("textbox", { name: "Resource ID" }),
      "https://host.example.com/a/b/c",
    );

    await user.click(
      screen.getByRole("button", {
        name: "Expand resource 1 into a hierarchy",
      }),
    );

    const typeInputs = screen.getAllByPlaceholderText("type");
    expect(typeInputs).toHaveLength(4);
    expect(typeInputs[0]).toHaveValue("Folder");
    expect(typeInputs[1]).toHaveValue("Folder");
    expect(typeInputs[2]).toHaveValue("Folder");
    expect(typeInputs[3]).toHaveValue("Folder");
  });

  it("expands a hierarchy into the configured parent_type", async () => {
    const user = userEvent.setup();
    renderDiagnostics();

    await user.type(
      screen.getByRole("combobox", { name: "Service" }),
      "document-service",
    );

    const typeInput = screen.getByRole("combobox", { name: "Resource type" });
    await user.click(typeInput);
    expect(
      await screen.findByRole("option", { name: "Document" }),
    ).toBeVisible();
    await user.type(typeInput, "Document");

    await user.type(
      screen.getByRole("textbox", { name: "Resource ID" }),
      "https://host.example.com/a/b/c",
    );

    await user.click(
      screen.getByRole("button", {
        name: "Expand resource 1 into a hierarchy",
      }),
    );

    const typeInputs = screen.getAllByPlaceholderText("type");
    expect(typeInputs).toHaveLength(4);
    expect(typeInputs[0]).toHaveValue("Document");
    expect(typeInputs[1]).toHaveValue("Folder");
    expect(typeInputs[2]).toHaveValue("Folder");
    expect(typeInputs[3]).toHaveValue("Folder");
  });

  it("submits and displays results grouped by resource", async () => {
    const user = userEvent.setup();
    renderDiagnostics();

    await fillRequiredFields(user);

    await user.click(screen.getByRole("button", { name: "Run diagnostics" }));

    expect(await screen.findByText("Results")).toBeVisible();
    expect(screen.getByText("doc-123")).toBeVisible();
    expect(screen.getByText("doc-456")).toBeVisible();
    expect(screen.getAllByText("Document").length).toBeGreaterThanOrEqual(2);
    expect(screen.getByText("permit")).toBeVisible();
    expect(screen.getByText("forbid")).toBeVisible();
  });

  it("renders policy IDs as links to policy details", async () => {
    const user = userEvent.setup();
    renderDiagnostics();

    await fillRequiredFields(user);

    await user.click(screen.getByRole("button", { name: "Run diagnostics" }));

    await waitFor(() => {
      expect(screen.getAllByRole("table").length).toBeGreaterThan(0);
    });

    const link = screen.getByRole("link", { name: "1" });
    expect(link).toHaveAttribute("href", "/policies/1");
  });

  it("shows error on API failure", async () => {
    server.use(
      http.post(BATCH_URL, () => {
        return new HttpResponse("Internal Server Error", { status: 500 });
      }),
    );

    const user = userEvent.setup();
    renderDiagnostics();

    await fillRequiredFields(user);

    await user.click(screen.getByRole("button", { name: "Run diagnostics" }));

    expect(await screen.findByText(/Internal Server Error/)).toBeVisible();
  });

  it("shows empty state when no results match", async () => {
    server.use(
      http.post(BATCH_URL, () => {
        return HttpResponse.json({ results: [] });
      }),
    );

    const user = userEvent.setup();
    renderDiagnostics();

    await fillRequiredFields(user);

    await user.click(screen.getByRole("button", { name: "Run diagnostics" }));

    expect(
      await screen.findByText("No results for the specified parameters."),
    ).toBeVisible();
  });

  it("resets the form when reset button is clicked", async () => {
    const user = userEvent.setup();
    renderDiagnostics();

    const principalInput = screen.getByRole("textbox", { name: "Principal" });
    const serviceInput = screen.getByRole("combobox", { name: "Service" });
    const resourceIdInput = screen.getByRole("textbox", { name: "Resource ID" });

    await user.type(principalInput, "user-123");
    await user.type(serviceInput, "document-service");
    await user.type(resourceIdInput, "doc-123");
    await user.click(screen.getByRole("button", { name: "Add resource" }));

    expect(screen.getAllByPlaceholderText("id")).toHaveLength(2);

    await user.click(screen.getByRole("button", { name: "Reset" }));

    expect(principalInput).toHaveValue("");
    expect(serviceInput).toHaveValue("");
    expect(screen.getAllByPlaceholderText("id")).toHaveLength(1);
    expect(screen.getByRole("textbox", { name: "Resource ID" })).toHaveValue(
      "",
    );
  });

  it("sends the correct batch payload", async () => {
    let capturedBody: Record<string, unknown> | undefined;
    server.use(
      http.post(BATCH_URL, async ({ request }) => {
        capturedBody = (await request.json()) as Record<string, unknown>;
        return HttpResponse.json(mockDiagnosticsBatchResponse);
      }),
    );

    const user = userEvent.setup();
    renderDiagnostics();

    await user.type(
      screen.getByRole("textbox", { name: "Principal" }),
      "user-123",
    );
    await fillRequiredFields(user);
    await user.type(
      screen.getByRole("combobox", { name: "Resource type" }),
      "Document",
    );
    await user.type(
      screen.getByRole("textbox", { name: "Resource ID" }),
      "doc-123",
    );
    fireEvent.change(screen.getByRole("textbox", { name: "Context" }), {
      target: { value: '{"ip": "10.0.0.1"}' },
    });

    await user.click(screen.getByRole("button", { name: "Run diagnostics" }));

    await waitFor(() => {
      expect(capturedBody).toBeDefined();
    });

    expect(capturedBody).toEqual({
      batches: [
        {
          principal: { sub: "user-123" },
          actions: [{ service: "document-service", name: "read" }],
          resource: { type: "Document", id: "doc-123" },
          context: { ip: "10.0.0.1" },
        },
      ],
    });
  });

  it("includes the principal's group membership as a groups claim", async () => {
    let capturedBody:
      | { batches: Array<Record<string, unknown>> }
      | undefined;
    server.use(
      http.post(BATCH_URL, async ({ request }) => {
        capturedBody = (await request.json()) as {
          batches: Array<Record<string, unknown>>;
        };
        return HttpResponse.json(mockDiagnosticsBatchResponse);
      }),
    );

    const user = userEvent.setup();
    render({
      initialEntries: ["/diagnostics"],
      children: <Diagnostics />,
      config: mockConfigWithUserService,
    });

    await user.type(
      screen.getByRole("textbox", { name: "Principal" }),
      "user-123",
    );
    await fillRequiredFields(user);

    await user.click(screen.getByRole("button", { name: "Run diagnostics" }));

    await waitFor(() => {
      expect(capturedBody).toBeDefined();
    });

    expect(capturedBody?.batches[0]).toEqual({
      principal: {
        sub: "user-123",
        info: { groups: ["group-123", "group-456"] },
      },
      actions: [{ service: "document-service", name: "read" }],
    });
  });

  it("caches group membership across submits for the same principal", async () => {
    let groupsRequests = 0;
    server.use(
      http.get(
        "http://localhost:9090/api/v1alpha/users/:userId/groups",
        () => {
          groupsRequests += 1;
          return HttpResponse.json({
            items: [{ id: "group-123" }],
            next_cursor: null,
          });
        },
      ),
      http.post(BATCH_URL, () =>
        HttpResponse.json(mockDiagnosticsBatchResponse),
      ),
    );

    const user = userEvent.setup();
    render({
      initialEntries: ["/diagnostics"],
      children: <Diagnostics />,
      config: mockConfigWithUserService,
    });

    await user.type(
      screen.getByRole("textbox", { name: "Principal" }),
      "user-123",
    );
    await fillRequiredFields(user);

    await user.click(screen.getByRole("button", { name: "Run diagnostics" }));
    await screen.findByText("Results");

    await user.click(screen.getByRole("button", { name: "Run diagnostics" }));
    await waitFor(() => {
      expect(screen.getByText("Results")).toBeVisible();
    });

    expect(groupsRequests).toBe(1);
  });

  it("warns and still runs when group membership cannot be fetched", async () => {
    server.use(
      http.get(
        "http://localhost:9090/api/v1alpha/users/:userId/groups",
        () => new HttpResponse("Internal Server Error", { status: 500 }),
      ),
      http.post(BATCH_URL, () =>
        HttpResponse.json(mockDiagnosticsBatchResponse),
      ),
    );

    const user = userEvent.setup();
    render({
      initialEntries: ["/diagnostics"],
      children: <Diagnostics />,
      config: mockConfigWithUserService,
    });

    await user.type(
      screen.getByRole("textbox", { name: "Principal" }),
      "user-123",
    );
    await fillRequiredFields(user);

    await user.click(screen.getByRole("button", { name: "Run diagnostics" }));

    expect(
      await screen.findByText("Group membership unavailable"),
    ).toBeVisible();
    expect(screen.getByText("Results")).toBeVisible();
  });

  it("omits optional fields and resource when no resource is specified", async () => {
    let capturedBody:
      | { batches: Array<Record<string, unknown>> }
      | undefined;
    server.use(
      http.post(BATCH_URL, async ({ request }) => {
        capturedBody = (await request.json()) as {
          batches: Array<Record<string, unknown>>;
        };
        return HttpResponse.json(mockDiagnosticsBatchResponse);
      }),
    );

    const user = userEvent.setup();
    renderDiagnostics();

    await fillRequiredFields(user);

    await user.click(screen.getByRole("button", { name: "Run diagnostics" }));

    await waitFor(() => {
      expect(capturedBody).toBeDefined();
    });

    expect(capturedBody?.batches).toHaveLength(1);
    expect(capturedBody?.batches[0]).toEqual({
      actions: [{ service: "document-service", name: "read" }],
    });
  });

  it("selects policies and opens the delete dialog", async () => {
    const user = userEvent.setup();
    renderDiagnostics();

    await fillRequiredFields(user);

    await user.click(screen.getByRole("button", { name: "Run diagnostics" }));

    await waitFor(() => {
      expect(screen.getByText("Results")).toBeVisible();
    });

    const deleteButton = screen.getByRole("button", {
      name: /Delete selected/,
    });
    expect(deleteButton).toBeDisabled();

    await user.click(
      screen.getByRole("checkbox", { name: "Select policy 1" }),
    );

    expect(
      screen.getByRole("button", { name: "Delete selected (1)" }),
    ).toBeEnabled();

    await user.click(
      screen.getByRole("button", { name: "Delete selected (1)" }),
    );

    const dialog = await screen.findByRole("dialog");
    expect(
      await within(dialog).findByText(
        /Are you sure you want to delete the following policy/,
      ),
    ).toBeInTheDocument();
    expect(within(dialog).getByText("#1")).toBeInTheDocument();
  });
});
