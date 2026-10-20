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
import { ServiceForm } from "../ServiceForm";
import { render } from "../../util/tests/render";
import { server } from "../../util/tests/msw";

describe("ServiceForm", () => {
  it("renders new service form with default values", async () => {
    render({
      initialEntries: ["/services/new"],
      children: <ServiceForm />,
    });

    expect(screen.getByText("Register service")).toBeVisible();

    const serviceInput = screen.getByRole("textbox", { name: "Name" });
    expect(serviceInput).toHaveValue("");

    const idClaimInput = screen.getByRole("textbox", {
      name: "Principal identifier",
    });
    expect(idClaimInput).toHaveValue("sub");

    expect(
      screen.getByText("Defines operations the service supports."),
    ).toBeVisible();
    expect(
      screen.getByText(/Defines resource types service operations use/),
    ).toBeVisible();

    expect(screen.getByRole("button", { name: "Submit" })).toBeVisible();
  });

  it("renders edit service form with initial values", async () => {
    const initialValues = {
      service: "test-service",
      idClaim: "email",
      actions: ["read", "write"],
      resources: [
        { type: "Document", priority: "permit" as const },
        { type: "Folder", priority: "forbid" as const },
      ],
    };

    render({
      initialEntries: ["/services/test-service/edit"],
      children: (
        <ServiceForm initialValues={initialValues} title="Edit service" />
      ),
    });

    expect(screen.getByText("Edit service")).toBeVisible();

    const serviceInput = screen.getByRole("textbox", { name: "Name" });
    expect(serviceInput).toHaveValue("test-service");

    const idClaimInput = screen.getByRole("textbox", {
      name: "Principal identifier",
    });
    expect(idClaimInput).toHaveValue("email");

    expect(screen.getByText("read")).toBeVisible();
    expect(screen.getByText("write")).toBeVisible();

    expect(screen.getByText("Document")).toBeVisible();
    expect(screen.getByText("Folder")).toBeVisible();
  });

  it("validates service name is required", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/services/new"],
      children: <ServiceForm />,
    });

    const submitButton = screen.getByRole("button", { name: "Submit" });
    await user.click(submitButton);

    expect(await screen.findByText("Service name is required.")).toBeVisible();
  });

  it("validates service name format", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/services/new"],
      children: <ServiceForm />,
    });

    const serviceInput = screen.getByRole("textbox", { name: "Name" });
    await user.type(serviceInput, "invalid service name!");

    const submitButton = screen.getByRole("button", { name: "Submit" });
    await user.click(submitButton);

    expect(
      await screen.findByText(
        "Service name must contain only alphanumeric characters, hyphens and underscores.",
      ),
    ).toBeVisible();
  });

  it("validates principal identifier is required", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/services/new"],
      children: <ServiceForm />,
    });

    const serviceInput = screen.getByRole("textbox", { name: "Name" });
    await user.type(serviceInput, "test-service");

    const idClaimInput = screen.getByRole("textbox", {
      name: "Principal identifier",
    });
    await user.clear(idClaimInput);

    const submitButton = screen.getByRole("button", { name: "Submit" });
    await user.click(submitButton);

    expect(
      await screen.findByText("Principal identifier is required."),
    ).toBeVisible();
  });

  it("validates action name format", async () => {
    server.use(
      http.put("http://localhost:8080/v1beta/services/test-service/", () => {
        return new HttpResponse("Server error", { status: 500 });
      }),
      http.put(
        "http://localhost:8080/v1beta/services/test-service/actions/",
        () => {
          return new HttpResponse("Server error", { status: 500 });
        },
      ),
      http.put(
        "http://localhost:8080/v1beta/services/test-service/resource-types/",
        () => {
          return new HttpResponse("Server error", { status: 500 });
        },
      ),
    );

    const user = userEvent.setup();
    render({
      initialEntries: ["/services/new"],
      children: <ServiceForm />,
    });

    const serviceInput = screen.getByRole("textbox", { name: "Name" });
    await user.type(serviceInput, "test-service");

    const actionInput = screen.getByPlaceholderText("New action name...");
    await user.type(actionInput, "InvalidAction");

    const addButton = screen.getByTitle("Add InvalidAction");
    await user.click(addButton);

    expect(await screen.findByText("InvalidAction")).toBeVisible();

    const submitButton = screen.getByRole("button", { name: "Submit" });
    await user.click(submitButton);

    expect(
      await screen.findByText(
        "Action name must contain only lowercased alphanumeric characters, hyphens and underscores.",
      ),
    ).toBeVisible();
  });

  it("validates resource type format", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/services/new"],
      children: <ServiceForm />,
    });

    const serviceInput = screen.getByRole("textbox", { name: "Name" });
    await user.type(serviceInput, "test-service");

    const resourceInput = screen.getByPlaceholderText("New resource type...");
    await user.type(resourceInput, "invalid@resource");

    const addButton = screen.getByTitle("Add invalid@resource");
    await user.click(addButton);

    expect(await screen.findByText("invalid@resource")).toBeVisible();

    const submitButton = screen.getByRole("button", { name: "Submit" });
    await user.click(submitButton);

    expect(
      await screen.findByText(
        "Resource type name must contain only alphanumeric characters.",
      ),
    ).toBeVisible();
  });

  it("adds and removes actions", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/services/new"],
      children: <ServiceForm />,
    });

    const actionInput = screen.getByPlaceholderText("New action name...");
    await user.type(actionInput, "read");

    const addButton = screen.getByTitle("Add read");
    await user.click(addButton);

    expect(await screen.findByText("read")).toBeVisible();

    const removeButton = screen.getByTitle("Remove read");
    await user.click(removeButton);

    expect(screen.queryByText("read")).not.toBeInTheDocument();
  });

  it("adds and removes resources with priority selection", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/services/new"],
      children: <ServiceForm />,
    });

    const resourceInput = screen.getByPlaceholderText("New resource type...");
    await user.type(resourceInput, "Document");

    const addButton = screen.getByTitle("Add Document");
    await user.click(addButton);

    expect(await screen.findByText("Document")).toBeVisible();

    const prioritySelect = screen.getByTitle("Document evaluation priority");
    expect(prioritySelect).toBeVisible();

    await user.click(prioritySelect);
    await user.click(screen.getByText("permit"));

    expect(prioritySelect).toHaveDisplayValue("permit");

    const removeButton = screen.getByTitle("Remove Document");
    await user.click(removeButton);

    expect(screen.queryByText("Document")).not.toBeInTheDocument();
  });

  it("submits service successfully", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/services/new"],
      children: <ServiceForm />,
    });

    const serviceInput = screen.getByRole("textbox", { name: "Name" });
    await user.type(serviceInput, "test-service");

    const actionInput = screen.getByPlaceholderText("New action name...");
    await user.type(actionInput, "read");
    const actionAddButton = screen.getByTitle("Add read");
    await user.click(actionAddButton);

    const resourceInput = screen.getByPlaceholderText("New resource type...");
    await user.type(resourceInput, "Document");
    const resourceAddButton = screen.getByTitle("Add Document");
    await user.click(resourceAddButton);

    const submitButton = screen.getByRole("button", { name: "Submit" });
    await user.click(submitButton);

    expect(
      await screen.findByText("Service test-service has been updated."),
    ).toBeVisible();
  });

  it("shows error when submission fails", async () => {
    server.use(
      http.put("http://localhost:8080/v1beta/services/test-service/", () => {
        return new HttpResponse("Internal Server Error", { status: 500 });
      }),
    );

    const user = userEvent.setup();
    render({
      initialEntries: ["/services/new"],
      children: <ServiceForm />,
    });

    const serviceInput = screen.getByRole("textbox", { name: "Name" });
    await user.type(serviceInput, "test-service");

    const submitButton = screen.getByRole("button", { name: "Submit" });
    await user.click(submitButton);

    expect(
      await screen.findByText(/Failed to submit the service/),
    ).toBeVisible();
  });

  it("disables form inputs while submitting", async () => {
    server.use(
      http.put(
        "http://localhost:8080/v1beta/services/test-service/",
        async () => {
          await new Promise((resolve) => setTimeout(resolve, 100));
          return new HttpResponse(null, { status: 200 });
        },
      ),
      http.put(
        "http://localhost:8080/v1beta/services/test-service/actions/",
        async () => {
          await new Promise((resolve) => setTimeout(resolve, 100));
          return new HttpResponse(null, { status: 200 });
        },
      ),
      http.put(
        "http://localhost:8080/v1beta/services/test-service/resource-types/",
        async () => {
          await new Promise((resolve) => setTimeout(resolve, 100));
          return new HttpResponse(null, { status: 200 });
        },
      ),
    );

    const user = userEvent.setup();
    render({
      initialEntries: ["/services/new"],
      children: <ServiceForm />,
    });

    const serviceInput = screen.getByRole("textbox", { name: "Name" });
    await user.type(serviceInput, "test-service");

    const submitButton = screen.getByRole("button", { name: "Submit" });
    await user.click(submitButton);

    expect(serviceInput).toBeDisabled();
    const idClaimInput = screen.getByRole("textbox", {
      name: "Principal identifier",
    });
    expect(idClaimInput).toBeDisabled();
    expect(submitButton).toBeDisabled();

    expect(
      await screen.findByText("Service test-service has been updated."),
    ).toBeVisible();
  });

  it("shows evaluation priority tooltip", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/services/new"],
      children: <ServiceForm />,
    });

    const resourceInput = screen.getByPlaceholderText("New resource type...");
    await user.type(resourceInput, "Document");

    const addButton = screen.getByTitle("Add Document");
    await user.click(addButton);

    const table = await screen.findByRole("table");
    expect(table).toBeVisible();

    expect(screen.getByText("Resource type")).toBeVisible();
    expect(screen.getByText("Evaluation priority")).toBeVisible();

    const tooltipIcon = screen.getByTitle("Evaluation priority help");
    expect(tooltipIcon).toBeVisible();

    await user.hover(tooltipIcon);

    await waitFor(() => {
      expect(
        screen.getByText(
          "Defines what decision must be prioritized when authorization has conflicting policies.",
        ),
      ).toBeVisible();
    });
  });
});
