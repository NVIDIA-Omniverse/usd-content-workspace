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
import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import { ServiceList } from "../ServiceList";
import { server } from "../../util/tests/msw";
import { render } from "../../util/tests/render";
import { mockNavigate } from "../../util/tests/router";

describe("ServiceList", () => {
  it("renders loading state initially", async () => {
    render({
      initialEntries: ["/services"],
      children: <ServiceList />,
    });

    expect(screen.getByText("Services")).toBeVisible();
    expect(
      screen.getByRole("button", { name: /register service/i }),
    ).toBeVisible();
    const loader = document.querySelector(".mantine-Loader-root");
    expect(loader).toBeInTheDocument();
  });

  it("renders services list after loading", async () => {
    render({
      initialEntries: ["/services"],
      children: <ServiceList />,
    });

    const table = await screen.findByRole("table");
    expect(table).toBeVisible();

    const rows = screen.getAllByRole("row");
    expect(rows).toHaveLength(4);

    expect(
      screen.getByRole("link", { name: "document-service" }),
    ).toBeVisible();
    expect(screen.getByTitle("Delete document-service")).toBeVisible();

    expect(screen.getByRole("link", { name: "user-service" })).toBeVisible();
    expect(screen.getByTitle("Delete user-service")).toBeVisible();

    expect(screen.getByRole("link", { name: "email-service" })).toBeVisible();
    expect(screen.getByTitle("Delete email-service")).toBeVisible();
  });

  it("renders Register service button with correct navigation", async () => {
    render({
      initialEntries: ["/services"],
      children: <ServiceList />,
    });

    const registerButton = screen.getByRole("link", {
      name: /register service/i,
    });
    expect(registerButton).toBeVisible();
    expect(registerButton).toHaveAttribute("href", "/services/new");
  });

  it("navigates to service details when row is clicked", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/services"],
      children: <ServiceList />,
    });

    const documentServiceRow = await screen.findByRole("link", {
      name: "document-service",
    });
    const row = documentServiceRow.closest("tr")!;
    expect(row).toHaveStyle("cursor: pointer");

    await user.click(row);

    expect(mockNavigate).toHaveBeenCalledTimes(1);
    expect(mockNavigate).toHaveBeenCalledWith("/services/document-service");
  });

  it("navigates to service details when service link is clicked", async () => {
    render({
      initialEntries: ["/services"],
      children: <ServiceList />,
    });

    const documentServiceLink = await screen.findByRole("link", {
      name: "document-service",
    });
    expect(documentServiceLink).toHaveAttribute(
      "href",
      "/services/document-service",
    );
  });

  it("opens delete confirmation modal when delete button is clicked", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/services"],
      children: <ServiceList />,
    });

    await screen.findByRole("table");

    const deleteButton = screen.getByTitle("Delete document-service");
    await user.click(deleteButton);

    const dialog = await screen.findByRole("dialog");
    expect(dialog).toBeInTheDocument();
    expect(
      await screen.findByText(/Delete document-service/),
    ).toBeInTheDocument();

    expect(
      screen.getByText(/Are you sure you want to delete service/),
    ).toBeInTheDocument();
    expect(within(dialog).getByText("document-service")).toBeInTheDocument();
    expect(
      screen.getByText(/This deletes all registered metadata/),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/This does not delete existing policies/),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/This action cannot be undone/),
    ).toBeInTheDocument();

    expect(screen.getByRole("button", { name: "Delete" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeInTheDocument();
  });

  it("prevents row click when delete button is clicked", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/services"],
      children: <ServiceList />,
    });

    await screen.findByRole("table");

    const deleteButton = screen.getByTitle("Delete document-service");
    await user.click(deleteButton);

    expect(mockNavigate).not.toHaveBeenCalled();
  });

  it("deletes service successfully and invalidates queries", async () => {
    const user = userEvent.setup();

    const services = [
      { service: "document-service", id_claim: "sub" },
      { service: "user-service", id_claim: "sub" },
      { service: "email-service", id_claim: "email" },
    ];

    server.use(
      http.delete(
        "http://localhost:8080/v1beta/services/:service",
        ({ params }) => {
          const index = services.findIndex((s) => s.service === params.service);
          if (index !== -1) {
            services.splice(index, 1);
          }
          return new HttpResponse(null, { status: 204 });
        },
      ),
      http.get("http://localhost:8080/v1beta/services/", () => {
        return HttpResponse.json(services);
      }),
    );

    render({
      initialEntries: ["/services"],
      children: <ServiceList />,
    });

    await screen.findByRole("table");

    const deleteButton = screen.getByTitle("Delete document-service");
    await user.click(deleteButton);

    const dialog = await screen.findByRole("dialog");
    const confirmButton = within(dialog).getByRole("button", {
      name: "Delete",
    });
    await user.click(confirmButton);

    const table = await screen.findByRole("table");
    expect(table).toBeVisible();

    expect(
      screen.queryByRole("link", { name: "document-service" }),
    ).not.toBeInTheDocument();
    expect(
      await screen.findByRole("link", { name: "user-service" }),
    ).toBeVisible();
    expect(screen.getByRole("link", { name: "email-service" })).toBeVisible();
  });

  it("shows error notification when delete fails", async () => {
    const user = userEvent.setup();

    server.use(
      http.delete("http://localhost:8080/v1beta/services/error-service", () => {
        return new HttpResponse("Internal Server Error", { status: 500 });
      }),
    );

    server.use(
      http.get("http://localhost:8080/v1beta/services/", () => {
        return HttpResponse.json([
          { service: "error-service", id_claim: "sub" },
        ]);
      }),
    );

    render({
      initialEntries: ["/services"],
      children: <ServiceList />,
    });

    await screen.findByRole("table");

    const deleteButton = screen.getByTitle("Delete error-service");
    await user.click(deleteButton);

    const dialog = await screen.findByRole("dialog");
    const confirmButton = within(dialog).getByRole("button", {
      name: "Delete",
    });
    await user.click(confirmButton);

    expect(await screen.findByRole("table")).toBeVisible();
  });

  it("renders error state when API fails", async () => {
    server.use(
      http.get("http://localhost:8080/v1beta/services/", () => {
        return new HttpResponse("Failed to fetch services", { status: 500 });
      }),
    );

    render({
      initialEntries: ["/services"],
      children: <ServiceList />,
    });

    expect(await screen.findByText(/Failed to fetch services/)).toBeVisible();
  });
});
