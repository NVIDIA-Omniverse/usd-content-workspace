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
import { describe, expect, it } from "vitest";
import { PolicyDetails } from "../PolicyDetails";
import { render } from "../../util/tests/render";
import { mockConfigWithUserService } from "../../util/tests/config";
import { mockNavigate } from "../../util/tests/router";
import { mockPermitPolicy, server } from "../../util/tests/msw";
import { Route, Routes } from "react-router-dom";

describe("PolicyDetails", () => {
  const routes = (
    <Routes>
      <Route path="/policies/:policyId" element={<PolicyDetails />} />
      <Route path="/policies" element={<>Policy list</>} />
    </Routes>
  );

  it("renders loading state initially", async () => {
    render({
      initialEntries: ["/policies/1"],
      children: routes,
    });

    expect(screen.getByText("Policy #1")).toBeVisible();
    const loader = document.querySelector(".mantine-Loader-root");
    expect(loader).toBeInTheDocument();
  });

  it("renders policy details after loading", async () => {
    render({
      initialEntries: ["/policies/1"],
      children: routes,
    });

    expect(await screen.findByText("Permit")).toBeVisible();
    expect(screen.getByText("user-123")).toBeVisible();
    expect(screen.getByText("document-service")).toBeVisible();
    expect(screen.getByText("read")).toBeVisible();
    expect(screen.getByText("Document")).toBeVisible();
    expect(screen.getByText("doc-123")).toBeVisible();
    expect(screen.getByText("Auto")).toBeVisible();

    const codeBlock = screen.getByText(/permit\(/);
    expect(codeBlock).toBeVisible();
  });

  it("renders order when set", async () => {
    server.use(
      http.get("http://localhost:8080/v1beta/policies/1", () => {
        return HttpResponse.json({ ...mockPermitPolicy, order: 5 });
      }),
    );

    render({
      initialEntries: ["/policies/1"],
      children: routes,
    });

    expect(await screen.findByText("5")).toBeVisible();
    expect(screen.queryByText("Auto")).not.toBeInTheDocument();
  });

  it("renders forbid policy correctly", async () => {
    render({
      initialEntries: ["/policies/2"],
      children: routes,
    });

    expect(await screen.findByText("Forbid")).toBeVisible();
    expect(screen.getByText("Unscoped")).toBeVisible();
    expect(screen.getByText("document-service")).toBeVisible();
    expect(screen.getByText("delete")).toBeVisible();

    const codeBlock = screen.getByText(/forbid\(/);
    expect(codeBlock).toBeVisible();
  });

  it("renders comment when set", async () => {
    render({
      initialEntries: ["/policies/1"],
      children: routes,
    });

    expect(await screen.findByText("Comment")).toBeVisible();
    expect(screen.getByText("owned by platform team")).toBeVisible();
  });

  it("renders em-dash placeholder when comment is null", async () => {
    render({
      initialEntries: ["/policies/2"],
      children: routes,
    });

    const header = await screen.findByText("Comment");
    const row = header.closest("tr")!;
    expect(within(row).getByText("\u2014")).toBeVisible();
  });

  it("renders error state when API fails", async () => {
    render({
      initialEntries: ["/policies/error"],
      children: routes,
    });

    const errorMessage = await screen.findByText(/Failed to fetch the policy/);
    expect(errorMessage).toBeVisible();
  });

  it("renders Edit button with correct navigation", async () => {
    render({
      initialEntries: ["/policies/1"],
      children: routes,
    });

    const editButton = await screen.findByRole("link", { name: /edit/i });
    expect(editButton).toBeVisible();
    expect(editButton).toHaveAttribute("href", "/policies/1/edit");
  });

  it("opens delete confirmation modal when Delete button is clicked", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies/1"],
      children: routes,
    });

    await screen.findByText("Permit");

    const deleteButton = screen.getByRole("button", { name: /delete/i });
    await user.click(deleteButton);

    const dialog = await screen.findByRole("dialog");
    expect(dialog).toBeInTheDocument();
    expect(
      await screen.findByText(/Are you sure you want to delete policy/),
    ).toBeInTheDocument();

    const buttons = screen.getAllByRole("button", { name: "Delete" });
    expect(buttons).toHaveLength(2);
    expect(screen.getByRole("button", { name: "Cancel" })).toBeInTheDocument();
  });

  it("deletes policy successfully and shows notification", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/policies/1"],
      children: routes,
    });

    await screen.findByText("Permit");

    const deleteButton = screen.getByRole("button", { name: /delete/i });
    await user.click(deleteButton);

    const dialog = await screen.findByRole("dialog");
    const confirmButtons = screen.getAllByRole("button", { name: "Delete" });
    const modalConfirmButton = confirmButtons.find((button) =>
      dialog.contains(button),
    );

    await user.click(modalConfirmButton!);

    await waitFor(() => {
      expect(mockNavigate).toHaveBeenCalledWith("/policies/");
    });

    await waitFor(() => {
      expect(
        screen.getByText(/Policy #1 has been deleted/),
      ).toBeInTheDocument();
    });
  });

  it("shows error notification when delete fails", async () => {
    const user = userEvent.setup();

    server.use(
      http.get("http://localhost:8080/v1beta/policies/error", () => {
        return HttpResponse.json(mockPermitPolicy);
      }),
    );

    render({
      initialEntries: ["/policies/error"],
      children: routes,
    });
    await screen.findByText("Permit");

    const deleteButton = screen.getByRole("button", { name: /delete/i });
    await user.click(deleteButton);

    const dialog = await screen.findByRole("dialog");
    const confirmButtons = screen.getAllByRole("button", { name: "Delete" });
    const modalConfirmButton = confirmButtons.find((button) =>
      dialog.contains(button),
    );

    await user.click(modalConfirmButton!);

    await waitFor(
      () => {
        expect(screen.getByText(/Failed to delete policy/)).toBeInTheDocument();
      },
      { timeout: 3000 },
    );
  });

  it("renders breadcrumbs correctly", async () => {
    render({
      initialEntries: ["/policies/1"],
      children: routes,
    });

    await screen.findByText("Permit");

    const policiesLink = screen.getByRole("link", { name: "Policies" });
    const currentPolicyLink = screen.getByRole("link", { name: "#1" });

    expect(policiesLink).toBeVisible();
    expect(policiesLink).toHaveAttribute("href", "/policies/");
    expect(currentPolicyLink).toBeVisible();
    expect(currentPolicyLink).toHaveAttribute("href", "/policies/1");
  });

  it("resolves principal to display name when user service is available", async () => {
    render({
      initialEntries: ["/policies/1"],
      config: mockConfigWithUserService,
      children: routes,
    });

    expect(await screen.findByText("Alice Johnson")).toBeVisible();
  });

  it("resolves created_by to display name when user service is available", async () => {
    server.use(
      http.get("http://localhost:8080/v1beta/policies/1", () => {
        return HttpResponse.json({
          ...mockPermitPolicy,
          created_by: "user-456",
        });
      }),
    );

    render({
      initialEntries: ["/policies/1"],
      config: mockConfigWithUserService,
      children: routes,
    });

    expect(await screen.findByText("Bob Smith")).toBeVisible();
  });

  it("shows raw created_by when user cannot be resolved", async () => {
    server.use(
      http.get("http://localhost:8080/v1beta/policies/1", () => {
        return HttpResponse.json({
          ...mockPermitPolicy,
          created_by: "unknown-user-id",
        });
      }),
    );

    render({
      initialEntries: ["/policies/1"],
      config: mockConfigWithUserService,
      children: routes,
    });

    expect(await screen.findByText("unknown-user-id")).toBeVisible();
  });
});
