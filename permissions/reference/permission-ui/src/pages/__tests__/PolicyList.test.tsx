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
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { afterEach, describe, expect, it, vi } from "vitest";
import { PolicyList } from "../PolicyList";
import {
  mockForbidPolicy,
  mockPermitPolicy,
  mockPolicyPage,
  server,
} from "../../util/tests/msw.ts";
import { render } from "../../util/tests/render.tsx";
import { mockNavigate } from "../../util/tests/router.ts";
import { mockConfigWithUserService } from "../../util/tests/config.ts";

describe("PolicyList", () => {
  afterEach(() => {
    localStorage.clear();
  });

  it("renders loading state initially", async () => {
    render({
      initialEntries: ["/policies"],
      children: <PolicyList />,
    });

    expect(screen.getByText("Policies")).toBeVisible();
    expect(screen.getByRole("button", { name: /search/i })).toBeVisible();
  });

  it("renders policies list after loading", async () => {
    render({
      initialEntries: ["/policies"],
      children: <PolicyList />,
    });

    const table = await screen.findByRole("table");
    expect(table).toBeVisible();

    const rows = screen.getAllByRole("row");
    expect(rows).toHaveLength(4);

    const policy1Link = screen.getByRole("link", { name: "1" });
    const policy1Row = policy1Link.closest("tr")!;

    expect(within(policy1Row).getByRole("link", { name: "1" })).toBeVisible();
    expect(within(policy1Row).getByText("Permit")).toBeVisible();
    expect(within(policy1Row).getByText("user-123")).toBeVisible();
    expect(within(policy1Row).getByText("document-service")).toBeVisible();
    expect(within(policy1Row).getByText("read")).toBeVisible();
    expect(within(policy1Row).getByText("Document")).toBeVisible();
    expect(within(policy1Row).getByText("doc-123")).toBeVisible();
    expect(
      within(policy1Row).getByText("owned by platform team"),
    ).toBeVisible();

    const policy2Link = screen.getByRole("link", { name: "2" });
    const policy2Row = policy2Link.closest("tr")!;

    expect(within(policy2Row).getByRole("link", { name: "2" })).toBeVisible();
    expect(within(policy2Row).getByText("Forbid")).toBeVisible();
    expect(within(policy2Row).getByText("Unscoped")).toBeVisible();
    expect(within(policy2Row).getByText("document-service")).toBeVisible();
    expect(within(policy2Row).getByText("delete")).toBeVisible();
    expect(within(policy2Row).getByText("Document")).toBeVisible();
    expect(within(policy2Row).getByText("doc-456")).toBeVisible();
    expect(
      within(policy2Row).queryByText("owned by platform team"),
    ).not.toBeInTheDocument();

    expect(within(policy1Row).getByText("Permit")).toBeVisible();
    expect(within(policy2Row).getByText("Forbid")).toBeVisible();
  });

  it("shows the full comment in a tooltip on hover", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies"],
      children: <PolicyList />,
    });

    const policy1Link = await screen.findByRole("link", { name: "1" });
    const policy1Row = policy1Link.closest("tr")!;
    const commentCell = within(policy1Row).getByText("owned by platform team");

    await user.hover(commentCell);

    const tooltip = await screen.findByRole("tooltip");
    expect(tooltip).toBeInTheDocument();
    expect(tooltip).toHaveTextContent("owned by platform team");
  });

  it("renders search form with all filter inputs", async () => {
    render({
      initialEntries: ["/policies"],
      children: <PolicyList />,
    });

    const searchButton = screen.getByRole("button", { name: /search/i });
    await userEvent.setup().click(searchButton);

    expect(await screen.findByLabelText("Principal")).toBeVisible();

    expect(screen.getByLabelText("Service")).toBeVisible();
    expect(screen.getByLabelText("Action")).toBeVisible();
    expect(screen.getByLabelText("Resource type")).toBeVisible();
    expect(screen.getByLabelText("Resource ID")).toBeVisible();
    expect(screen.getByLabelText("Order")).toBeVisible();
  });

  it("submits search form with filter values", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies"],
      children: <PolicyList />,
    });

    const searchButton = screen.getByRole("button", { name: /search/i });
    await user.click(searchButton);

    expect(await screen.findByLabelText("Principal")).toBeVisible();

    const principalInput = screen.getByLabelText("Principal");
    await user.type(principalInput, "user-123");

    const submitButton = screen.getByRole("button", { name: /find policies/i });
    await user.click(submitButton);

    const table = await screen.findByRole("table");
    expect(table).toBeVisible();

    const rows = screen.getAllByRole("row");
    expect(rows).toHaveLength(2);

    const policy1Link = screen.getByRole("link", { name: "1" });
    const policy1Row = policy1Link.closest("tr")!;

    expect(within(policy1Row).getByRole("link", { name: "1" })).toBeVisible();
    expect(within(policy1Row).getByText("Permit")).toBeVisible();
    expect(within(policy1Row).getByText("user-123")).toBeVisible();
    expect(within(policy1Row).getByText("document-service")).toBeVisible();
    expect(within(policy1Row).getByText("read")).toBeVisible();
    expect(within(policy1Row).getByText("Document")).toBeVisible();
    expect(within(policy1Row).getByText("doc-123")).toBeVisible();

    expect(screen.queryByRole("link", { name: "2" })).not.toBeInTheDocument();
  });

  it("sends the group filter when the principal type is set to group", async () => {
    let capturedGroup: string | null = null;
    let capturedPrincipal: string | null = null;
    server.use(
      http.get("http://localhost:8080/v1beta/policies/", ({ request }) => {
        const url = new URL(request.url);
        capturedGroup = url.searchParams.get("group");
        capturedPrincipal = url.searchParams.get("principal");
        return HttpResponse.json(mockPolicyPage);
      }),
    );

    const user = userEvent.setup();
    render({
      initialEntries: ["/policies"],
      config: mockConfigWithUserService,
      children: <PolicyList />,
    });

    const searchButton = screen.getByRole("button", { name: /search/i });
    await user.click(searchButton);

    await screen.findByLabelText("Principal");

    await user.click(screen.getByLabelText("Principal type: User"));
    await user.click(await screen.findByRole("menuitem", { name: "Group" }));

    await user.type(screen.getByLabelText("Principal"), "group-123");

    await user.click(screen.getByRole("button", { name: /find policies/i }));

    await waitFor(() => {
      expect(capturedGroup).toBe("group-123");
    });
    expect(capturedPrincipal).toBeNull();
  });

  it("submits search form with order filter value", async () => {
    server.use(
      http.get("http://localhost:8080/v1beta/policies/", ({ request }) => {
        const order = new URL(request.url).searchParams.get("order");
        const items = [
          { ...mockPermitPolicy, order: 10 },
          { ...mockForbidPolicy, order: 20 },
        ].filter((policy) => !order || policy.order === Number(order));
        return HttpResponse.json({ items, page: 1, page_size: 50 });
      }),
    );

    const user = userEvent.setup();
    render({
      initialEntries: ["/policies"],
      children: <PolicyList />,
    });

    const searchButton = screen.getByRole("button", { name: /search/i });
    await user.click(searchButton);

    const orderInput = await screen.findByLabelText("Order");
    await user.type(orderInput, "10");

    const submitButton = screen.getByRole("button", { name: /find policies/i });
    await user.click(submitButton);

    const table = await screen.findByRole("table");
    expect(table).toBeVisible();

    expect(screen.getByRole("link", { name: "1" })).toBeVisible();
    expect(screen.queryByRole("link", { name: "2" })).not.toBeInTheDocument();
  });

  it("sends order=0 as a filter to the API", async () => {
    let capturedOrder: string | null = null;
    server.use(
      http.get("http://localhost:8080/v1beta/policies/", ({ request }) => {
        capturedOrder = new URL(request.url).searchParams.get("order");
        return HttpResponse.json({ items: [], page: 1, page_size: 50 });
      }),
    );

    const user = userEvent.setup();
    render({
      initialEntries: ["/policies"],
      children: <PolicyList />,
    });

    const searchButton = screen.getByRole("button", { name: /search/i });
    await user.click(searchButton);

    const orderInput = await screen.findByLabelText("Order");
    await user.type(orderInput, "0");

    const submitButton = screen.getByRole("button", { name: /find policies/i });
    await user.click(submitButton);

    await waitFor(() => {
      expect(capturedOrder).toBe("0");
    });
  });

  it("prevents entering non-numeric or negative order values", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies"],
      children: <PolicyList />,
    });

    const searchButton = screen.getByRole("button", { name: /search/i });
    await user.click(searchButton);

    const orderInput = await screen.findByLabelText("Order");
    await user.type(orderInput, "-12ab3");

    expect(orderInput).toHaveValue("123");
  });

  it("preserves order search parameter from URL", async () => {
    render({
      initialEntries: ["/policies?order=20"],
      children: <PolicyList />,
    });

    const searchButton = screen.getByRole("button", { name: /search/i });
    await userEvent.setup().click(searchButton);

    expect(await screen.findByDisplayValue("20")).toBeVisible();
  });

  it("validates service and action dependency", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies"],
      children: <PolicyList />,
    });

    await waitFor(() => {
      expect(screen.queryByRole("progressbar")).not.toBeInTheDocument();
    });

    const searchButton = screen.getByRole("button", { name: /search/i });
    await user.click(searchButton);

    const actionInput = screen.getByLabelText("Action");
    await user.type(actionInput, "read");

    const submitButton = screen.getByRole("button", { name: /find policies/i });
    await user.click(submitButton);

    expect(
      await screen.findByText("Service is required if action is specified."),
    ).toBeVisible();
  });

  it("validates resource type and resource ID dependency", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies"],
      children: <PolicyList />,
    });

    const searchButton = screen.getByRole("button", { name: /search/i });
    await user.click(searchButton);

    const resourceIdInput = screen.getByLabelText("Resource ID");
    await user.type(resourceIdInput, "doc-123");

    const submitButton = screen.getByRole("button", { name: /find policies/i });
    await user.click(submitButton);

    expect(
      await screen.findByText(
        "Resource type is required if resource ID is specified.",
      ),
    ).toBeVisible();
  });

  it("handles NULL values correctly in form", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies"],
      children: <PolicyList />,
    });

    const searchButton = screen.getByRole("button", { name: /search/i });
    await user.click(searchButton);

    const serviceInput = screen.getByLabelText("Service");
    const actionInput = screen.getByLabelText("Action");

    await user.type(serviceInput, "NULL");
    await user.type(actionInput, "read");

    const submitButton = screen.getByRole("button", { name: /find policies/i });
    await user.click(submitButton);

    await waitFor(() => {
      expect(actionInput).toHaveValue("");
    });
  });

  it("navigates to policy details when row is clicked", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies"],
      children: <PolicyList />,
    });

    const policy1Link = await screen.findByRole("link", { name: "1" });
    const policyRow = policy1Link.closest("tr")!;
    expect(policyRow).toHaveStyle("cursor: pointer");

    await user.click(policyRow);

    expect(mockNavigate).toHaveBeenCalledTimes(1);
    expect(mockNavigate).toHaveBeenCalledWith("/policies/1");
  });

  it("does not navigate when text is selected in the table row", async () => {
    const user = userEvent.setup();
    const mockGetSelection = vi.fn();
    Object.defineProperty(window, "getSelection", {
      value: mockGetSelection,
      writable: true,
    });

    render({
      initialEntries: ["/policies"],
      children: <PolicyList />,
    });

    const policy1Link = await screen.findByRole("link", { name: "1" });
    const policyRow = policy1Link.closest("tr")!;

    mockGetSelection.mockReturnValue({
      toString: () => "selected text",
    });

    await user.click(policyRow);

    expect(mockNavigate).toHaveBeenCalledTimes(0);

    mockGetSelection.mockReturnValue({
      toString: () => "",
    });

    await user.click(policyRow);

    expect(mockNavigate).toHaveBeenCalledTimes(1);
    expect(mockNavigate).toHaveBeenCalledWith("/policies/1");
  });

  it("renders New Policy button", async () => {
    render({
      initialEntries: ["/policies"],
      children: <PolicyList />,
    });

    const newPolicyButton = screen.getByRole("link", { name: /new policy/i });
    expect(newPolicyButton).toBeVisible();
    expect(newPolicyButton).toHaveAttribute("href", "/policies/new");
  });

  it("handles API errors gracefully", async () => {
    server.use(
      http.get("http://localhost:8080/v1beta/policies/", () => {
        return new HttpResponse("Internal Server Error", { status: 500 });
      }),
    );

    render({
      initialEntries: ["/policies"],
      children: <PolicyList />,
    });

    expect(await screen.findByText(/Failed to fetch policies/)).toBeVisible();
  });

  it("renders empty state when no policies found", async () => {
    server.use(
      http.get("http://localhost:8080/v1beta/policies/", () => {
        return HttpResponse.json({
          items: [],
          page: 1,
          page_size: 50,
        });
      }),
    );

    render({
      initialEntries: ["/policies"],
      children: <PolicyList />,
    });

    expect(await screen.findByText("No results found")).toBeVisible();
    expect(screen.queryByRole("table")).not.toBeInTheDocument();
  });

  it("preserves search parameters from URL", async () => {
    render({
      initialEntries: [
        "/policies?page=1&principal=test-user&service=test-service",
      ],
      children: <PolicyList />,
    });

    const searchButton = screen.getByRole("button", { name: /search/i });
    await userEvent.setup().click(searchButton);

    expect(await screen.findByDisplayValue("test-user")).toBeVisible();
    expect(await screen.findByDisplayValue("test-service")).toBeVisible();
  });

  it("applies the principal filter from a direct link with an empty group param", async () => {
    let capturedPrincipal: string | null = null;
    let capturedGroup: string | null = null;
    server.use(
      http.get("http://localhost:8080/v1beta/policies/", ({ request }) => {
        const url = new URL(request.url);
        capturedPrincipal = url.searchParams.get("principal");
        capturedGroup = url.searchParams.get("group");
        return HttpResponse.json(mockPolicyPage);
      }),
    );

    render({
      initialEntries: [
        "/policies?principal=user-123&principalType=user&service=&action=&resourceType=&resourceId=&order=&group=",
      ],
      children: <PolicyList />,
    });

    await waitFor(() => {
      expect(capturedPrincipal).toBe("user-123");
    });
    expect(capturedGroup).toBeNull();
  });

  it("loads autocomplete data for services", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies"],
      children: <PolicyList />,
    });

    const searchButton = screen.getByRole("button", { name: /search/i });
    await user.click(searchButton);

    const serviceInput = await screen.findByLabelText("Service");
    expect(serviceInput).toHaveAttribute("aria-haspopup", "listbox");

    await user.click(serviceInput);

    expect(screen.getAllByText("document-service")).toHaveLength(4);
    expect(screen.getByText("user-service")).toBeInTheDocument();
  });

  it("trims form values before submission", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies"],
      children: <PolicyList />,
    });

    const searchButton = screen.getByRole("button", { name: /search/i });
    await user.click(searchButton);

    const principalInput = screen.getByLabelText("Principal");
    await user.type(principalInput, "  user-123  ");

    const submitButton = screen.getByRole("button", { name: /find policies/i });
    await user.click(submitButton);

    await waitFor(() => {
      expect(principalInput).toHaveValue("user-123");
    });
  });

  it("renders a selection checkbox per row and a select-all checkbox", async () => {
    render({
      initialEntries: ["/policies"],
      children: <PolicyList />,
    });

    await screen.findByRole("table");

    expect(
      screen.getByRole("checkbox", { name: "Select all policies" }),
    ).toBeVisible();
    expect(
      screen.getByRole("checkbox", { name: "Select policy 1" }),
    ).toBeVisible();
    expect(
      screen.getByRole("checkbox", { name: "Select policy 2" }),
    ).toBeVisible();
  });

  it("enables the delete button only when policies are selected", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies"],
      children: <PolicyList />,
    });

    await screen.findByRole("table");

    const deleteButton = screen.getByRole("button", {
      name: /delete selected/i,
    });
    expect(deleteButton).toBeDisabled();

    await user.click(screen.getByRole("checkbox", { name: "Select policy 1" }));

    expect(deleteButton).toBeEnabled();
  });

  it("selects and deselects all policies with the header checkbox", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies"],
      children: <PolicyList />,
    });

    await screen.findByRole("table");

    const selectAll = screen.getByRole("checkbox", {
      name: "Select all policies",
    });
    const policy1 = screen.getByRole("checkbox", { name: "Select policy 1" });
    const policy2 = screen.getByRole("checkbox", { name: "Select policy 2" });

    await user.click(selectAll);
    expect(policy1).toBeChecked();
    expect(policy2).toBeChecked();

    await user.click(selectAll);
    expect(policy1).not.toBeChecked();
    expect(policy2).not.toBeChecked();
  });

  it("does not navigate when a row checkbox is clicked", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies"],
      children: <PolicyList />,
    });

    await screen.findByRole("table");

    await user.click(screen.getByRole("checkbox", { name: "Select policy 1" }));

    expect(mockNavigate).toHaveBeenCalledTimes(0);
  });

  it("toggles selection when clicking the checkbox cell outside the checkbox", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies"],
      children: <PolicyList />,
    });

    await screen.findByRole("table");

    const checkbox = screen.getByRole("checkbox", { name: "Select policy 1" });
    const cell = checkbox.closest("td")!;

    await user.click(cell);

    expect(mockNavigate).toHaveBeenCalledTimes(0);
    expect(
      screen.getByRole("checkbox", { name: "Select policy 1" }),
    ).toBeChecked();
    expect(
      screen.getByRole("button", { name: /delete selected/i }),
    ).toBeEnabled();
  });


  it("shows a confirmation dialog with policy text for each selected policy", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies"],
      children: <PolicyList />,
    });

    await screen.findByRole("table");

    await user.click(screen.getByRole("checkbox", { name: "Select all policies" }));
    await user.click(
      screen.getByRole("button", { name: /delete selected/i }),
    );

    const dialog = await screen.findByRole("dialog");

    expect(within(dialog).getAllByText(/permit\(/)).toHaveLength(2);
    expect(within(dialog).getByText(/forbid\(/)).toBeInTheDocument();

    expect(within(dialog).getAllByText("Permit")).toHaveLength(2);
    expect(within(dialog).getByText("Forbid")).toBeInTheDocument();
    expect(within(dialog).getByText("#1")).toBeInTheDocument();
    expect(within(dialog).getByText("#2")).toBeInTheDocument();
    expect(within(dialog).getByText("#3")).toBeInTheDocument();

    expect(
      within(dialog).getByText(/This action cannot be undone/),
    ).toBeInTheDocument();
  });

  it("resolves principal display names inside the confirmation dialog", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies"],
      config: mockConfigWithUserService,
      children: <PolicyList />,
    });

    await screen.findByRole("table");

    await user.click(screen.getByRole("checkbox", { name: "Select policy 1" }));
    await user.click(
      screen.getByRole("button", { name: /delete selected/i }),
    );

    const dialog = await screen.findByRole("dialog");
    expect(
      await within(dialog).findByText("Alice Johnson"),
    ).toBeInTheDocument();
  });

  it("deletes selected policies and shows a success notification", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies"],
      children: <PolicyList />,
    });

    await screen.findByRole("table");

    await user.click(screen.getByRole("checkbox", { name: "Select all policies" }));
    await user.click(
      screen.getByRole("button", { name: /delete selected/i }),
    );

    const dialog = await screen.findByRole("dialog");
    const confirmButton = within(dialog).getByRole("button", { name: "Delete" });
    await user.click(confirmButton);

    await waitFor(() => {
      expect(screen.getByText(/Deleted 3 policies/)).toBeInTheDocument();
    });
  });

  it("shows a failure status, alert, and retry button when deletion fails", async () => {
    const user = userEvent.setup();

    server.use(
      http.delete("http://localhost:8080/v1beta/policies/:policyId", () => {
        return new HttpResponse("Internal Server Error", { status: 500 });
      }),
    );

    render({
      initialEntries: ["/policies"],
      children: <PolicyList />,
    });

    await screen.findByRole("table");

    await user.click(screen.getByRole("checkbox", { name: "Select policy 1" }));
    await user.click(
      screen.getByRole("button", { name: /delete selected/i }),
    );

    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Delete" }));

    await waitFor(
      () => {
        expect(screen.getByText(/Failed to delete 1 policy/)).toBeInTheDocument();
      },
      { timeout: 3000 },
    );

    expect(
      within(dialog).getByLabelText("Policy #1 failed to delete"),
    ).toBeInTheDocument();
    expect(
      within(dialog).getByText("Failed to delete policy #1"),
    ).toBeInTheDocument();
    expect(
      within(dialog).getByRole("button", { name: "Retry" }),
    ).toBeInTheDocument();
  });

  it("shows per-policy success icons after deletion", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies"],
      children: <PolicyList />,
    });

    await screen.findByRole("table");

    await user.click(screen.getByRole("checkbox", { name: "Select all policies" }));
    await user.click(
      screen.getByRole("button", { name: /delete selected/i }),
    );

    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Delete" }));

    expect(
      await within(dialog).findByLabelText("Policy #1 deleted"),
    ).toBeInTheDocument();
    expect(
      await within(dialog).findByLabelText("Policy #2 deleted"),
    ).toBeInTheDocument();
    expect(
      within(dialog).getByRole("button", { name: "Close" }),
    ).toBeInTheDocument();
  });

  it("retries only the failed policies", async () => {
    const user = userEvent.setup();

    const deleteCalls: string[] = [];
    server.use(
      http.delete(
        "http://localhost:8080/v1beta/policies/:policyId",
        ({ params }) => {
          deleteCalls.push(String(params.policyId));
          if (params.policyId === "1") {
            return new HttpResponse("Internal Server Error", { status: 500 });
          }
          return new HttpResponse(null, { status: 204 });
        },
      ),
    );

    render({
      initialEntries: ["/policies"],
      children: <PolicyList />,
    });

    await screen.findByRole("table");

    await user.click(screen.getByRole("checkbox", { name: "Select all policies" }));
    await user.click(
      screen.getByRole("button", { name: /delete selected/i }),
    );

    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Delete" }));

    expect(
      await within(dialog).findByLabelText("Policy #2 deleted"),
    ).toBeInTheDocument();
    expect(
      within(dialog).getByLabelText("Policy #1 failed to delete"),
    ).toBeInTheDocument();

    expect(deleteCalls).toEqual(["1", "2", "3"]);

    server.use(
      http.delete("http://localhost:8080/v1beta/policies/:policyId", () => {
        return new HttpResponse(null, { status: 204 });
      }),
    );

    await user.click(within(dialog).getByRole("button", { name: "Retry" }));

    expect(
      await within(dialog).findByLabelText("Policy #1 deleted"),
    ).toBeInTheDocument();

    expect(deleteCalls.filter((id) => id === "2")).toHaveLength(1);
  });

  it("renders the default columns and hides the optional ones", async () => {
    render({
      initialEntries: ["/policies"],
      children: <PolicyList />,
    });

    await screen.findByRole("table");

    const headers = screen
      .getAllByRole("columnheader")
      .map((header) => header.textContent);

    expect(headers).toEqual([
      "",
      "ID",
      "Policy",
      "Principal",
      "Service",
      "Action",
      "Resource type",
      "Resource ID",
      "Comment",
    ]);

    expect(
      screen.queryByRole("columnheader", { name: "Order" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("columnheader", { name: "Created by" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("columnheader", { name: "Created at" }),
    ).not.toBeInTheDocument();
  });

  it("shows an optional column after enabling it in the picker", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies"],
      children: <PolicyList />,
    });

    await screen.findByRole("table");

    await user.click(screen.getByRole("button", { name: /columns/i }));
    await user.click(
      await screen.findByRole("checkbox", { name: "Created by" }),
    );

    expect(
      await screen.findByRole("columnheader", { name: "Created by" }),
    ).toBeVisible();

    const policy1Row = screen.getByRole("link", { name: "1" }).closest("tr")!;
    expect(within(policy1Row).getByText("alice@example.com")).toBeVisible();
  });

  it("resolves the created_by column to a display name when user service is available", async () => {
    server.use(
      http.get("http://localhost:8080/v1beta/policies/", () => {
        return HttpResponse.json({
          ...mockPolicyPage,
          items: [{ ...mockPermitPolicy, created_by: "user-456" }],
        });
      }),
    );

    const user = userEvent.setup();
    render({
      initialEntries: ["/policies"],
      config: mockConfigWithUserService,
      children: <PolicyList />,
    });

    await screen.findByRole("table");

    await user.click(screen.getByRole("button", { name: /columns/i }));
    await user.click(
      await screen.findByRole("checkbox", { name: "Created by" }),
    );

    const policy1Row = screen.getByRole("link", { name: "1" }).closest("tr")!;
    expect(await within(policy1Row).findByText("Bob Smith")).toBeVisible();
  });

  it("hides a default column after disabling it in the picker", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies"],
      children: <PolicyList />,
    });

    await screen.findByRole("table");

    expect(
      screen.getByRole("columnheader", { name: "Comment" }),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /columns/i }));
    await user.click(await screen.findByRole("checkbox", { name: "Comment" }));

    await waitFor(() => {
      expect(
        screen.queryByRole("columnheader", { name: "Comment" }),
      ).not.toBeInTheDocument();
    });
  });

  it("reorders columns with the up control in the picker", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies"],
      children: <PolicyList />,
    });

    await screen.findByRole("table");

    await user.click(screen.getByRole("button", { name: /columns/i }));
    await user.click(
      await screen.findByRole("button", { name: "Move Service up" }),
    );

    await waitFor(() => {
      const headers = screen
        .getAllByRole("columnheader")
        .map((header) => header.textContent);
      expect(headers).toEqual([
        "",
        "ID",
        "Policy",
        "Service",
        "Principal",
        "Action",
        "Resource type",
        "Resource ID",
        "Comment",
      ]);
    });
  });

  it("restores the persisted column configuration from local storage", async () => {
    localStorage.setItem(
      "policy-list-columns",
      JSON.stringify({
        order: ["created_by", "id", "effect"],
        visible: ["created_by", "id"],
      }),
    );

    render({
      initialEntries: ["/policies"],
      children: <PolicyList />,
    });

    await screen.findByRole("table");

    await waitFor(() => {
      const headers = screen
        .getAllByRole("columnheader")
        .map((header) => header.textContent);
      expect(headers).toEqual(["", "Created by", "ID"]);
    });
  });
});
