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
import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import { PolicyForm } from "../PolicyForm";
import { render } from "../../util/tests/render";
import { mockConfigWithUserService } from "../../util/tests/config";
import { mockPermitPolicy, server } from "../../util/tests/msw";

describe("PolicyForm", () => {
  it("renders new policy form with default values", async () => {
    render({
      initialEntries: ["/policies/new"],
      children: <PolicyForm />,
    });

    expect(screen.getByText("New policy")).toBeVisible();

    const serviceInput = screen.getByRole("combobox", { name: "Service" });
    expect(serviceInput).toHaveValue("");

    const actionInput = screen.getByRole("combobox", { name: "Action" });
    expect(actionInput).toHaveValue("");

    const resourceTypeInput = screen.getByRole("combobox", {
      name: "Resource type",
    });
    expect(resourceTypeInput).toHaveValue("");

    const resourceIdInput = screen.getByRole("textbox", {
      name: "Resource ID",
    });
    expect(resourceIdInput).toHaveValue("");

    const principalInput = screen.getByRole("textbox", { name: "Principal" });
    expect(principalInput).toHaveValue("");

    const effectSelect = screen.getByRole("combobox", { name: "Effect" });
    expect(effectSelect).toHaveDisplayValue("Permit");

    const conditionsSelect = screen.getByRole("combobox", {
      name: "Conditions (advanced)",
    });
    expect(conditionsSelect).toHaveDisplayValue("None");

    const orderInput = screen.getByRole("textbox", { name: "Order" });
    expect(orderInput).toHaveValue("");

    const commentInput = screen.getByRole("textbox", { name: "Comment" });
    expect(commentInput).toHaveValue("");

    expect(screen.getByRole("button", { name: "Submit" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Reset" })).toBeVisible();
  });

  it("renders edit policy form with initial values", async () => {
    render({
      initialEntries: ["/policies/1/edit"],
      children: <PolicyForm initial={mockPermitPolicy} />,
    });

    expect(screen.getByText("Policy #1")).toBeVisible();

    const serviceInput = screen.getByRole("combobox", { name: "Service" });
    expect(serviceInput).toHaveValue("document-service");

    const actionInput = screen.getByRole("combobox", { name: "Action" });
    expect(actionInput).toHaveValue("read");

    const resourceTypeInput = screen.getByRole("combobox", {
      name: "Resource type",
    });
    expect(resourceTypeInput).toHaveValue("Document");

    const resourceIdInput = screen.getByRole("textbox", {
      name: "Resource ID",
    });
    expect(resourceIdInput).toHaveValue("doc-123");

    const principalInput = screen.getByRole("textbox", { name: "Principal" });
    expect(principalInput).toHaveValue("user-123");

    const effectSelect = screen.getByRole("combobox", { name: "Effect" });
    expect(effectSelect).toHaveDisplayValue("Permit");

    const conditionsSelect = screen.getByRole("combobox", {
      name: "Conditions (advanced)",
    });
    expect(conditionsSelect).toHaveDisplayValue("None");

    const commentInput = screen.getByRole("textbox", { name: "Comment" });
    expect(commentInput).toHaveValue("owned by platform team");
  });

  it("renders edit form with initial order", async () => {
    render({
      initialEntries: ["/policies/1/edit"],
      children: <PolicyForm initial={{ ...mockPermitPolicy, order: 10 }} />,
    });

    const orderInput = screen.getByRole("textbox", { name: "Order" });
    expect(orderInput).toHaveValue("10");
  });

  it("renders breadcrumbs correctly", async () => {
    render({
      initialEntries: ["/policies/new"],
      children: <PolicyForm />,
    });

    const policiesLink = screen.getByRole("link", { name: "Policies" });
    expect(policiesLink).toBeVisible();
    expect(policiesLink).toHaveAttribute("href", "/policies/");
  });

  it("renders breadcrumbs for edit form", async () => {
    render({
      initialEntries: ["/policies/1/edit"],
      children: <PolicyForm initial={mockPermitPolicy} />,
    });

    const policiesLink = screen.getByRole("link", { name: "Policies" });
    const policyLink = screen.getByRole("link", { name: "#1" });
    expect(policiesLink).toBeVisible();
    expect(policiesLink).toHaveAttribute("href", "/policies/");
    expect(policyLink).toBeVisible();
    expect(policyLink).toHaveAttribute("href", "/policies/1");
  });

  it("validates service field", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies/new"],
      children: <PolicyForm />,
    });

    const serviceInput = screen.getByRole("combobox", { name: "Service" });
    await user.type(serviceInput, "invalid_service");

    const submitButton = screen.getByRole("button", { name: "Submit" });
    await user.click(submitButton);

    expect(
      await screen.findByText(
        "Service name must only contain alphanumeric characters and dashes.",
      ),
    ).toBeVisible();
  });

  it("validates action field", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies/new"],
      children: <PolicyForm />,
    });

    const actionInput = screen.getByRole("combobox", { name: "Action" });
    await user.type(actionInput, "invalid_action");

    // Trigger validation by blurring the field
    await user.click(actionInput);
    await user.tab();

    const submitButton = screen.getByRole("button", { name: "Submit" });
    await user.click(submitButton);

    expect(
      await screen.findByText(
        "Action must only contain alphanumeric characters and dashes.",
      ),
    ).toBeVisible();
  });

  it("validates resource type field", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies/new"],
      children: <PolicyForm />,
    });

    const resourceTypeInput = screen.getByRole("combobox", {
      name: "Resource type",
    });
    await user.type(resourceTypeInput, "invalid_resource_type");

    // Trigger validation by blurring the field
    await user.click(resourceTypeInput);
    await user.tab();

    const submitButton = screen.getByRole("button", { name: "Submit" });
    await user.click(submitButton);

    expect(
      await screen.findByText(
        "Resource type must only contain alphanumeric characters and dashes.",
      ),
    ).toBeVisible();
  });

  it("validates resource ID field", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies/new"],
      children: <PolicyForm />,
    });

    const resourceIdInput = screen.getByLabelText("Resource ID");
    await user.type(resourceIdInput, 'resource-id"with-quotes');

    const submitButton = screen.getByRole("button", { name: "Submit" });
    await user.click(submitButton);

    expect(
      await screen.findByText("Resource ID must not contain quotes."),
    ).toBeVisible();
  });

  it("validates service and action dependency", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies/new"],
      children: <PolicyForm />,
    });

    const actionInput = screen.getByRole("combobox", { name: "Action" });
    await user.type(actionInput, "read");

    // Trigger validation by blurring the field
    await user.click(actionInput);
    await user.tab();

    const submitButton = screen.getByRole("button", { name: "Submit" });
    await user.click(submitButton);

    expect(
      await screen.findByText("Service is required if action is specified."),
    ).toBeVisible();
  });

  it("validates action required when service is provided", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies/new"],
      children: <PolicyForm />,
    });

    const serviceInput = screen.getByRole("combobox", { name: "Service" });
    await user.type(serviceInput, "document-service");

    // Trigger validation by blurring the field
    await user.click(serviceInput);
    await user.tab();

    const submitButton = screen.getByRole("button", { name: "Submit" });
    await user.click(submitButton);

    expect(
      await screen.findByText("Action is required if service is specified."),
    ).toBeVisible();
  });

  it("validates resource type and resource ID dependency", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies/new"],
      children: <PolicyForm />,
    });

    const resourceIdInput = screen.getByLabelText("Resource ID");
    await user.type(resourceIdInput, "resource-123");

    const submitButton = screen.getByRole("button", { name: "Submit" });
    await user.click(submitButton);

    expect(
      await screen.findByText(
        "Resource type is required if resource ID is specified.",
      ),
    ).toBeVisible();
  });

  it("validates resource ID required when resource type is provided", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies/new"],
      children: <PolicyForm />,
    });

    const resourceTypeInput = screen.getByRole("combobox", {
      name: "Resource type",
    });
    await user.type(resourceTypeInput, "Document");

    // Trigger validation by blurring the field
    await user.click(resourceTypeInput);
    await user.tab();

    const submitButton = screen.getByRole("button", { name: "Submit" });
    await user.click(submitButton);

    expect(
      await screen.findByText(
        "Resource ID is required if resource type is specified.",
      ),
    ).toBeVisible();
  });

  it("validates conditions are required when clause is specified", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies/new"],
      children: <PolicyForm />,
    });

    const conditionsSelect = screen.getByRole("combobox", {
      name: "Conditions (advanced)",
    });
    await user.click(conditionsSelect);
    await user.click(screen.getByText("When"));

    const serviceInput = screen.getByRole("combobox", { name: "Service" });
    await user.type(serviceInput, "test-service");

    const submitButton = screen.getByRole("button", { name: "Submit" });
    await user.click(submitButton);

    expect(
      await screen.findByText("Conditions must be specified."),
    ).toBeVisible();
  });

  it("shows error if no scopes or conditions are specified", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies/new"],
      children: <PolicyForm />,
    });

    const submitButton = screen.getByRole("button", { name: "Submit" });
    await user.click(submitButton);

    expect(
      await screen.findByText(
        "At least one scope or condition must be specified.",
      ),
    ).toBeVisible();
  });

  it("shows warning when no principal is specified", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies/new"],
      children: <PolicyForm />,
    });

    const serviceInput = screen.getByRole("combobox", { name: "Service" });
    await user.type(serviceInput, "test-service");

    expect(
      await screen.findByText(
        "This policy does not include a principal scope -- it will be applied for all users.",
      ),
    ).toBeVisible();
  });

  it("shows warning when no service is specified", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies/new"],
      children: <PolicyForm />,
    });

    const principalInput = screen.getByLabelText("Principal");
    await user.type(principalInput, "user-123");

    expect(
      await screen.findByText(
        "This policy does not include a service scope -- it will be applied for all operations for all services.",
      ),
    ).toBeVisible();
  });

  it("shows URL warning for resource ID", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies/new"],
      children: <PolicyForm />,
    });

    const resourceIdInput = screen.getByLabelText("Resource ID");
    await user.type(resourceIdInput, "https://example.com/resource");

    expect(
      await screen.findByText(
        "This resource ID is a URL -- each path segment will be percent-encoded.",
      ),
    ).toBeVisible();
  });

  it("submits resource ID verbatim while previewing the server-encoded form (regression: no re-encoding on save)", async () => {
    // Regression for the double-encoding cascade `%20 -> %2520 -> %252520 ...`
    // on each Save. The request body must carry the user's exact bytes (both
    // `resource.id` and the Cedar `policy` text); the preview separately
    // mirrors the server's path-segment encoding so the user sees what will
    // be stored.
    let capturedBody: Record<string, unknown> | undefined;
    server.use(
      http.put(
        "http://localhost:8080/v1beta/policies/",
        async ({ request }) => {
          capturedBody = (await request.json()) as Record<string, unknown>;
          return HttpResponse.json({ ...mockPermitPolicy, id: 9 });
        },
      ),
    );

    const canonical =
      "https://example.blob.core.windows.net/Rohan%20-Service-permission-test/";

    const user = userEvent.setup();
    render({
      initialEntries: ["/policies/new"],
      children: <PolicyForm />,
    });

    await user.type(
      screen.getByRole("combobox", { name: "Service" }),
      "document-service",
    );
    await user.type(screen.getByRole("combobox", { name: "Action" }), "read");
    await user.type(
      screen.getByRole("combobox", { name: "Resource type" }),
      "Document",
    );
    await user.type(screen.getByLabelText("Resource ID"), canonical);
    await user.type(
      screen.getByRole("textbox", { name: "Principal" }),
      "user-123",
    );

    // Preview mirrors the once-encoded form the service will store. For an
    // already canonical `%20` input that means the preview equals the input.
    const preview = screen.getByLabelText("Preview");
    expect(preview).toHaveTextContent(`resource == Document::"${canonical}"`);
    expect(preview).not.toHaveTextContent("%2520");

    await user.click(screen.getByRole("button", { name: "Submit" }));

    await waitFor(() => {
      expect(capturedBody).toBeDefined();
    });
    expect((capturedBody?.resource as { id: string }).id).toBe(canonical);
    expect(capturedBody?.policy).toContain(`Document::"${canonical}"`);
    expect(capturedBody?.policy).not.toContain("%2520");
  });

  it("encodes raw spaces in resource ID path segments for the preview", async () => {
    let capturedBody: Record<string, unknown> | undefined;
    server.use(
      http.put(
        "http://localhost:8080/v1beta/policies/",
        async ({ request }) => {
          capturedBody = (await request.json()) as Record<string, unknown>;
          return HttpResponse.json({ ...mockPermitPolicy, id: 10 });
        },
      ),
    );

    const user = userEvent.setup();
    render({
      initialEntries: ["/policies/new"],
      children: <PolicyForm />,
    });

    await user.type(
      screen.getByRole("combobox", { name: "Service" }),
      "storage",
    );
    await user.type(screen.getByRole("combobox", { name: "Action" }), "read");
    await user.type(
      screen.getByRole("combobox", { name: "Resource type" }),
      "File",
    );
    await user.type(
      screen.getByLabelText("Resource ID"),
      "http://test/abc def g/",
    );

    const preview = screen.getByLabelText("Preview");
    expect(preview).toHaveTextContent(
      `resource == File::"http://test/abc%20def%20g/"`,
    );
    expect(preview).not.toHaveTextContent("%2520");

    await user.click(screen.getByRole("button", { name: "Submit" }));

    await waitFor(() => {
      expect(capturedBody).toBeDefined();
    });
    expect((capturedBody?.resource as { id: string }).id).toBe(
      "http://test/abc def g/",
    );
  });

  it("loads service autocomplete data", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies/new"],
      children: <PolicyForm />,
    });

    const serviceInput = screen.getByRole("combobox", { name: "Service" });
    await user.click(serviceInput);

    await waitFor(() => {
      expect(screen.getByText("document-service")).toBeInTheDocument();
    });
    expect(screen.getByText("user-service")).toBeInTheDocument();
  });

  it("shows actions autocomplete when service is selected", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies/new"],
      children: <PolicyForm />,
    });

    const serviceInput = screen.getByRole("combobox", { name: "Service" });
    await user.type(serviceInput, "document-service");

    await waitFor(() => {
      const actionInput = screen.getByRole("combobox", { name: "Action" });
      expect(actionInput).toBeInTheDocument();
    });

    const actionInput = screen.getByRole("combobox", { name: "Action" });
    expect(actionInput).toBeInTheDocument();
  });

  it("shows resource types autocomplete when service is selected", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies/new"],
      children: <PolicyForm />,
    });

    const serviceInput = screen.getByRole("combobox", { name: "Service" });
    await user.type(serviceInput, "document-service");

    await waitFor(() => {
      const resourceTypeInput = screen.getByRole("combobox", {
        name: "Resource type",
      });
      expect(resourceTypeInput).toBeInTheDocument();
    });

    expect(screen.getByText("Resource type")).toBeInTheDocument();
  });

  it("shows conditions textarea when clause is selected", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies/new"],
      children: <PolicyForm />,
    });

    const conditionsSelect = screen.getByRole("combobox", {
      name: "Conditions (advanced)",
    });
    await user.click(conditionsSelect);
    await user.click(screen.getByText("When"));

    await waitFor(() => {
      const textareas = screen.getAllByRole("textbox");
      expect(textareas.length).toBeGreaterThan(0);
    });
  });

  it("generates correct policy preview", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies/new"],
      children: <PolicyForm />,
    });

    const serviceInput = screen.getByRole("combobox", { name: "Service" });
    const actionInput = screen.getByRole("combobox", { name: "Action" });
    const resourceTypeInput = screen.getByRole("combobox", {
      name: "Resource type",
    });
    const resourceIdInput = screen.getByRole("textbox", {
      name: "Resource ID",
    });
    const principalInput = screen.getByRole("textbox", { name: "Principal" });

    await user.type(serviceInput, "document-service");
    await user.type(actionInput, "read");
    await user.type(resourceTypeInput, "Document");
    await user.type(resourceIdInput, "doc-123");
    await user.type(principalInput, "user-123");

    await waitFor(() => {
      expect(screen.getByText(/permit\(/)).toBeInTheDocument();
    });

    const previewElement = screen.getByLabelText("Preview");
    expect(previewElement).toHaveTextContent(
      'permit( principal == Principal::"user-123", action == Action::"document-service:read", resource == Document::"doc-123" );',
    );
  });

  it("generates forbid policy preview", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies/new"],
      children: <PolicyForm />,
    });

    const effectSelect = screen.getByRole("combobox", { name: "Effect" });
    await user.click(effectSelect);
    await user.click(screen.getByText("Forbid"));

    const serviceInput = screen.getByRole("combobox", { name: "Service" });
    await user.type(serviceInput, "document-service");

    const previewElement = screen.getByLabelText("Preview");
    expect(previewElement).toHaveTextContent(
      "forbid( principal, action, resource );",
    );
  });

  it("resets form when reset button is clicked", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies/new"],
      children: <PolicyForm />,
    });

    const serviceInput = screen.getByRole("combobox", { name: "Service" });
    await user.type(serviceInput, "test-service");
    expect(serviceInput).toHaveValue("test-service");

    const actionInput = screen.getByRole("combobox", { name: "Action" });
    await user.type(actionInput, "test-action");
    expect(actionInput).toHaveValue("test-action");

    const resourceTypeInput = screen.getByRole("combobox", {
      name: "Resource type",
    });
    await user.type(resourceTypeInput, "TestResource");
    expect(resourceTypeInput).toHaveValue("TestResource");

    const resourceIdInput = screen.getByRole("textbox", {
      name: "Resource ID",
    });
    await user.type(resourceIdInput, "test-123");
    expect(resourceIdInput).toHaveValue("test-123");

    const principalInput = screen.getByRole("textbox", { name: "Principal" });
    await user.type(principalInput, "test-user");
    expect(principalInput).toHaveValue("test-user");

    const effectSelect = screen.getByRole("combobox", { name: "Effect" });
    await user.click(effectSelect);
    await user.click(screen.getByText("Forbid"));
    expect(effectSelect).toHaveDisplayValue("Forbid");

    const orderInput = screen.getByRole("textbox", { name: "Order" });
    await user.type(orderInput, "5");
    expect(orderInput).toHaveValue("5");

    const commentInput = screen.getByRole("textbox", { name: "Comment" });
    await user.type(commentInput, "reviewed by alice");
    expect(commentInput).toHaveValue("reviewed by alice");

    const resetButton = screen.getByRole("button", { name: "Reset" });
    await user.click(resetButton);

    expect(serviceInput).toHaveValue("");
    expect(actionInput).toHaveValue("");
    expect(resourceTypeInput).toHaveValue("");
    expect(resourceIdInput).toHaveValue("");
    expect(principalInput).toHaveValue("");
    expect(effectSelect).toHaveDisplayValue("Permit");
    expect(orderInput).toHaveValue("");
    expect(commentInput).toHaveValue("");

    const conditionsSelect = screen.getByRole("combobox", {
      name: "Conditions (advanced)",
    });
    expect(conditionsSelect).toHaveDisplayValue("None");
  });

  it("creates new policy successfully", async () => {
    server.use(
      http.put("http://localhost:8080/v1beta/policies/", () => {
        return HttpResponse.json({ ...mockPermitPolicy, id: 3 });
      }),
    );

    const user = userEvent.setup();
    render({
      initialEntries: ["/policies/new"],
      children: <PolicyForm />,
    });

    const serviceInput = screen.getByRole("combobox", { name: "Service" });
    const actionInput = screen.getByRole("combobox", { name: "Action" });
    const principalInput = screen.getByRole("textbox", { name: "Principal" });

    await user.type(serviceInput, "document-service");
    await user.type(actionInput, "read");
    await user.type(principalInput, "user-123");

    const submitButton = screen.getByRole("button", { name: "Submit" });
    await user.click(submitButton);

    await waitFor(() => {
      expect(
        screen.getByText(/Policy #3 has been created/),
      ).toBeInTheDocument();
    });
  });

  it("submits order when specified", async () => {
    let capturedBody: Record<string, unknown> | undefined;
    server.use(
      http.put(
        "http://localhost:8080/v1beta/policies/",
        async ({ request }) => {
          capturedBody = (await request.json()) as Record<string, unknown>;
          return HttpResponse.json({ ...mockPermitPolicy, id: 5, order: 3 });
        },
      ),
    );

    const user = userEvent.setup();
    render({
      initialEntries: ["/policies/new"],
      children: <PolicyForm />,
    });

    await user.type(
      screen.getByRole("combobox", { name: "Service" }),
      "document-service",
    );
    await user.type(screen.getByRole("combobox", { name: "Action" }), "read");
    await user.type(
      screen.getByRole("textbox", { name: "Principal" }),
      "user-123",
    );
    await user.type(screen.getByRole("textbox", { name: "Order" }), "3");

    await user.click(screen.getByRole("button", { name: "Submit" }));

    await waitFor(() => {
      expect(capturedBody).toBeDefined();
    });
    expect(capturedBody?.order).toBe(3);
  });

  it("omits order when not specified", async () => {
    let capturedBody: Record<string, unknown> | undefined;
    server.use(
      http.put(
        "http://localhost:8080/v1beta/policies/",
        async ({ request }) => {
          capturedBody = (await request.json()) as Record<string, unknown>;
          return HttpResponse.json({ ...mockPermitPolicy, id: 6 });
        },
      ),
    );

    const user = userEvent.setup();
    render({
      initialEntries: ["/policies/new"],
      children: <PolicyForm />,
    });

    await user.type(
      screen.getByRole("combobox", { name: "Service" }),
      "document-service",
    );
    await user.type(screen.getByRole("combobox", { name: "Action" }), "read");
    await user.type(
      screen.getByRole("textbox", { name: "Principal" }),
      "user-123",
    );

    await user.click(screen.getByRole("button", { name: "Submit" }));

    await waitFor(() => {
      expect(capturedBody).toBeDefined();
    });
    expect(capturedBody?.order).toBeUndefined();
  });

  it("submits comment when specified", async () => {
    let capturedBody: Record<string, unknown> | undefined;
    server.use(
      http.put(
        "http://localhost:8080/v1beta/policies/",
        async ({ request }) => {
          capturedBody = (await request.json()) as Record<string, unknown>;
          return HttpResponse.json({
            ...mockPermitPolicy,
            id: 7,
            comment: "reviewed by alice",
          });
        },
      ),
    );

    const user = userEvent.setup();
    render({
      initialEntries: ["/policies/new"],
      children: <PolicyForm />,
    });

    await user.type(
      screen.getByRole("combobox", { name: "Service" }),
      "document-service",
    );
    await user.type(screen.getByRole("combobox", { name: "Action" }), "read");
    await user.type(
      screen.getByRole("textbox", { name: "Principal" }),
      "user-123",
    );
    await user.type(
      screen.getByRole("textbox", { name: "Comment" }),
      "reviewed by alice",
    );

    await user.click(screen.getByRole("button", { name: "Submit" }));

    await waitFor(() => {
      expect(capturedBody).toBeDefined();
    });
    expect(capturedBody?.comment).toBe("reviewed by alice");
  });

  it("omits comment when not specified", async () => {
    let capturedBody: Record<string, unknown> | undefined;
    server.use(
      http.put(
        "http://localhost:8080/v1beta/policies/",
        async ({ request }) => {
          capturedBody = (await request.json()) as Record<string, unknown>;
          return HttpResponse.json({ ...mockPermitPolicy, id: 8 });
        },
      ),
    );

    const user = userEvent.setup();
    render({
      initialEntries: ["/policies/new"],
      children: <PolicyForm />,
    });

    await user.type(
      screen.getByRole("combobox", { name: "Service" }),
      "document-service",
    );
    await user.type(screen.getByRole("combobox", { name: "Action" }), "read");
    await user.type(
      screen.getByRole("textbox", { name: "Principal" }),
      "user-123",
    );

    await user.click(screen.getByRole("button", { name: "Submit" }));

    await waitFor(() => {
      expect(capturedBody).toBeDefined();
    });
    expect(capturedBody?.comment).toBeUndefined();
  });

  it("updates existing policy successfully", async () => {
    let deleteCalled = false;
    server.use(
      http.put("http://localhost:8080/v1beta/policies/", () => {
        return HttpResponse.json({ ...mockPermitPolicy, id: 4 });
      }),
      http.delete("http://localhost:8080/v1beta/policies/1", () => {
        deleteCalled = true;
        return new HttpResponse(null, { status: 204 });
      }),
    );

    const user = userEvent.setup();
    render({
      initialEntries: ["/policies/1/edit"],
      children: <PolicyForm initial={mockPermitPolicy} />,
    });

    const principalInput = screen.getByRole("textbox", { name: "Principal" });
    await user.clear(principalInput);
    await user.type(principalInput, "user-456");

    const submitButton = screen.getByRole("button", { name: "Submit" });
    await user.click(submitButton);

    await waitFor(() => {
      expect(
        screen.getByText(/Policy #1 has been replaced with new policy/),
      ).toBeInTheDocument();
    });
    expect(deleteCalled).toBe(true);
  });

  it("does not delete original policy when service dedup returns the same id", async () => {
    // When the canonical policy hash matches, the service refreshes the
    // existing row's mutable metadata (order, comment, audit fields) and
    // returns the same id. The form must treat this as an in-place update
    // and not delete the row we just upserted.
    let deleteCalled = false;
    server.use(
      http.put("http://localhost:8080/v1beta/policies/", () => {
        return HttpResponse.json({
          ...mockPermitPolicy,
          comment: "updated note",
        });
      }),
      http.delete("http://localhost:8080/v1beta/policies/1", () => {
        deleteCalled = true;
        return new HttpResponse(null, { status: 204 });
      }),
    );

    const user = userEvent.setup();
    render({
      initialEntries: ["/policies/1/edit"],
      children: <PolicyForm initial={mockPermitPolicy} />,
    });

    const commentInput = screen.getByRole("textbox", { name: "Comment" });
    await user.clear(commentInput);
    await user.type(commentInput, "updated note");

    const submitButton = screen.getByRole("button", { name: "Submit" });
    await user.click(submitButton);

    await waitFor(() => {
      expect(
        screen.getByText(/Policy #1 has been updated/),
      ).toBeInTheDocument();
    });
    expect(deleteCalled).toBe(false);
  });

  it("shows error when creation fails", async () => {
    server.use(
      http.put("http://localhost:8080/v1beta/policies/", () => {
        return new HttpResponse("Internal Server Error", { status: 500 });
      }),
    );

    const user = userEvent.setup();
    render({
      initialEntries: ["/policies/new"],
      children: <PolicyForm />,
    });

    const serviceInput = screen.getByRole("combobox", { name: "Service" });
    const actionInput = screen.getByRole("combobox", { name: "Action" });
    const principalInput = screen.getByRole("textbox", { name: "Principal" });

    await user.type(serviceInput, "document-service");
    await user.type(actionInput, "read");
    await user.type(principalInput, "user-123");

    const submitButton = screen.getByRole("button", { name: "Submit" });
    await user.click(submitButton);

    expect(await screen.findByText(/Internal Server Error/)).toBeVisible();
  });

  it("validates email format for principal when service has email id_claim", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies/new"],
      children: <PolicyForm />,
    });

    const serviceInput = screen.getByRole("combobox", { name: "Service" });
    await user.type(serviceInput, "email-service");

    const actionInput = screen.getByRole("combobox", { name: "Action" });
    await user.type(actionInput, "read");

    const principalInput = screen.getByRole("textbox", { name: "Principal" });
    await user.type(principalInput, "invalid-email");

    expect(await screen.findByText(/should be a valid email/)).toBeVisible();
  });

  it("shows unresolved principal warning when user service is configured and principal is not resolved", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies/new"],
      config: mockConfigWithUserService,
      children: <PolicyForm />,
    });

    const serviceInput = screen.getByRole("combobox", { name: "Service" });
    await user.type(serviceInput, "document-service");

    const actionInput = screen.getByRole("combobox", { name: "Action" });
    await user.type(actionInput, "read");

    const principalInput = screen.getByRole("textbox", { name: "Principal" });
    await user.type(principalInput, "some-unknown-value");

    expect(
      await screen.findByText(/could not be resolved to a known user/),
    ).toBeVisible();
  });

  it("does not show unresolved principal warning when user selects from autocomplete", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies/new"],
      config: mockConfigWithUserService,
      children: <PolicyForm />,
    });

    const serviceInput = screen.getByRole("combobox", { name: "Service" });
    await user.type(serviceInput, "document-service");

    const actionInput = screen.getByRole("combobox", { name: "Action" });
    await user.type(actionInput, "read");

    const principalInput = screen.getByRole("textbox", { name: "Principal" });
    await user.type(principalInput, "Alice");

    const option = await screen.findByText("Alice Johnson");
    await user.click(option);

    await waitFor(() => {
      expect(
        screen.queryByText(/could not be resolved to a known user/),
      ).not.toBeInTheDocument();
    });
  });

  it("does not show unresolved principal warning when idClaim is email", async () => {
    const user = userEvent.setup();

    server.use(
      http.get("http://localhost:8080/v1beta/services/:service/meta/", () => {
        return HttpResponse.json({ id_claim: "email" });
      }),
    );

    render({
      initialEntries: ["/policies/new"],
      config: mockConfigWithUserService,
      children: <PolicyForm />,
    });

    const serviceInput = screen.getByRole("combobox", { name: "Service" });
    await user.type(serviceInput, "email-service");

    const actionInput = screen.getByRole("combobox", { name: "Action" });
    await user.type(actionInput, "read");

    const principalInput = screen.getByRole("textbox", { name: "Principal" });
    await user.type(principalInput, "some-typed-value");

    await waitFor(() => {
      expect(
        screen.queryByText(/could not be resolved to a known user/),
      ).not.toBeInTheDocument();
    });
  });

  it("compiles a group-scoped Cedar head when the group type is selected", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies/new"],
      children: <PolicyForm />,
    });

    await user.click(screen.getByLabelText("Principal type: User"));
    await user.click(await screen.findByRole("menuitem", { name: "Group" }));

    await user.type(
      screen.getByRole("textbox", { name: "Principal" }),
      "group-123",
    );

    const preview = screen.getByLabelText("Preview");
    expect(preview).toHaveTextContent('principal in Group::"group-123"');
    expect(preview).not.toHaveTextContent("Principal::");
  });

  it("submits a group instead of a principal for group-scoped policies", async () => {
    let capturedBody: Record<string, unknown> | undefined;
    server.use(
      http.put(
        "http://localhost:8080/v1beta/policies/",
        async ({ request }) => {
          capturedBody = (await request.json()) as Record<string, unknown>;
          return HttpResponse.json({ ...mockPermitPolicy, id: 11 });
        },
      ),
    );

    const user = userEvent.setup();
    render({
      initialEntries: ["/policies/new"],
      children: <PolicyForm />,
    });

    await user.click(screen.getByLabelText("Principal type: User"));
    await user.click(await screen.findByRole("menuitem", { name: "Group" }));

    await user.type(
      screen.getByRole("combobox", { name: "Service" }),
      "document-service",
    );
    await user.type(screen.getByRole("combobox", { name: "Action" }), "read");
    await user.type(
      screen.getByRole("textbox", { name: "Principal" }),
      "group-123",
    );

    await user.click(screen.getByRole("button", { name: "Submit" }));

    await waitFor(() => {
      expect(capturedBody).toBeDefined();
    });
    expect(capturedBody?.group).toBe("group-123");
    expect(capturedBody?.principal).toBeUndefined();
    expect(capturedBody?.policy).toContain('principal in Group::"group-123"');
  });

  it("initializes the form with the group type when editing a group policy", async () => {
    render({
      initialEntries: ["/policies/1/edit"],
      children: (
        <PolicyForm
          initial={{
            ...mockPermitPolicy,
            principal: null,
            group: { id: "group-123" },
          }}
        />
      ),
    });

    expect(screen.getByLabelText("Principal type: Group")).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "Principal" })).toHaveValue(
      "group-123",
    );
  });

  it("does not show unresolved principal warning when user service is not configured", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies/new"],
      children: <PolicyForm />,
    });

    const serviceInput = screen.getByRole("combobox", { name: "Service" });
    await user.type(serviceInput, "document-service");

    const actionInput = screen.getByRole("combobox", { name: "Action" });
    await user.type(actionInput, "read");

    const principalInput = screen.getByRole("textbox", { name: "Principal" });
    await user.type(principalInput, "some-unknown-value");

    await waitFor(() => {
      expect(
        screen.queryByText(/could not be resolved to a known user/),
      ).not.toBeInTheDocument();
    });
  });

  it("defaults the resource operator to equality for a new policy", async () => {
    render({
      initialEntries: ["/policies/new"],
      children: <PolicyForm />,
    });

    expect(
      screen.getByLabelText("Resource operator: =="),
    ).toBeInTheDocument();
  });

  it("compiles a prefix-scoped Cedar head when the in operator is selected", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies/new"],
      children: <PolicyForm />,
    });

    await user.type(
      screen.getByRole("combobox", { name: "Resource type" }),
      "Document",
    );
    await user.type(screen.getByLabelText("Resource ID"), "doc-123");

    await user.click(screen.getByLabelText("Resource operator: =="));
    await user.click(
      await screen.findByRole("menuitem", { name: /under a prefix/i }),
    );

    expect(screen.getByLabelText("Resource operator: in")).toBeInTheDocument();

    const preview = screen.getByLabelText("Preview");
    expect(preview).toHaveTextContent('resource in Document::"doc-123"');
    expect(preview).not.toHaveTextContent('resource == Document::');
  });

  it("submits resource_prefix instead of resource for prefix-scoped policies", async () => {
    let capturedBody: Record<string, unknown> | undefined;
    server.use(
      http.put(
        "http://localhost:8080/v1beta/policies/",
        async ({ request }) => {
          capturedBody = (await request.json()) as Record<string, unknown>;
          return HttpResponse.json({ ...mockPermitPolicy, id: 12 });
        },
      ),
    );

    const user = userEvent.setup();
    render({
      initialEntries: ["/policies/new"],
      children: <PolicyForm />,
    });

    await user.type(
      screen.getByRole("combobox", { name: "Resource type" }),
      "Document",
    );
    await user.type(screen.getByLabelText("Resource ID"), "doc-123");

    await user.click(screen.getByLabelText("Resource operator: =="));
    await user.click(
      await screen.findByRole("menuitem", { name: /under a prefix/i }),
    );

    await user.click(screen.getByRole("button", { name: "Submit" }));

    await waitFor(() => {
      expect(capturedBody).toBeDefined();
    });
    expect(capturedBody?.resource_prefix).toEqual({
      id: "doc-123",
      type: "Document",
    });
    expect(capturedBody?.resource).toBeUndefined();
    expect(capturedBody?.policy).toContain('resource in Document::"doc-123"');
  });

  it("submits resource instead of resource_prefix for equality-scoped policies", async () => {
    let capturedBody: Record<string, unknown> | undefined;
    server.use(
      http.put(
        "http://localhost:8080/v1beta/policies/",
        async ({ request }) => {
          capturedBody = (await request.json()) as Record<string, unknown>;
          return HttpResponse.json({ ...mockPermitPolicy, id: 13 });
        },
      ),
    );

    const user = userEvent.setup();
    render({
      initialEntries: ["/policies/new"],
      children: <PolicyForm />,
    });

    await user.type(
      screen.getByRole("combobox", { name: "Resource type" }),
      "Document",
    );
    await user.type(screen.getByLabelText("Resource ID"), "doc-123");

    await user.click(screen.getByRole("button", { name: "Submit" }));

    await waitFor(() => {
      expect(capturedBody).toBeDefined();
    });
    expect(capturedBody?.resource).toEqual({
      id: "doc-123",
      type: "Document",
    });
    expect(capturedBody?.resource_prefix).toBeUndefined();
    expect(capturedBody?.policy).toContain('resource == Document::"doc-123"');
  });

  it("initializes the form with the in operator when editing a resource_prefix policy", async () => {
    render({
      initialEntries: ["/policies/1/edit"],
      children: (
        <PolicyForm
          initial={{
            ...mockPermitPolicy,
            resource: null,
            resource_prefix: { id: "doc-123", type: "Document", data: null },
          }}
        />
      ),
    });

    expect(screen.getByLabelText("Resource operator: in")).toBeInTheDocument();
    expect(
      screen.getByRole("combobox", { name: "Resource type" }),
    ).toHaveValue("Document");
    expect(screen.getByRole("textbox", { name: "Resource ID" })).toHaveValue(
      "doc-123",
    );
  });
});
