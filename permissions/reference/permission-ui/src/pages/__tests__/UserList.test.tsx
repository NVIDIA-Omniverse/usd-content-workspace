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
import { UserList } from "../UserList";
import { server } from "../../util/tests/msw";
import { render } from "../../util/tests/render";
import { mockConfigWithUserService } from "../../util/tests/config";

describe("UserList", () => {
  it("renders loading state initially", () => {
    render({
      initialEntries: ["/users"],
      config: mockConfigWithUserService,
      children: <UserList />,
    });

    expect(screen.getByText("Users")).toBeVisible();
    const loader = document.querySelector(".mantine-Loader-root");
    expect(loader).toBeInTheDocument();
  });

  it("renders users list after loading", async () => {
    render({
      initialEntries: ["/users"],
      config: mockConfigWithUserService,
      children: <UserList />,
    });

    expect(await screen.findByRole("link", { name: "Alice Johnson" })).toBeVisible();
    expect(screen.getByRole("link", { name: "Bob Smith" })).toBeVisible();
    expect(screen.getByRole("link", { name: "Charlie Brown" })).toBeVisible();
  });

  it("searches users by identifier", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/users"],
      config: mockConfigWithUserService,
      children: <UserList />,
    });

    await screen.findByRole("link", { name: "Alice Johnson" });

    await user.click(screen.getByRole("button", { name: "Search" }));
    await user.type(
      await screen.findByRole("textbox", { name: "Identifier" }),
      "user-456",
    );
    await user.click(screen.getByRole("button", { name: /find users/i }));

    expect(await screen.findByRole("link", { name: "Bob Smith" })).toBeVisible();
    await waitFor(() =>
      expect(
        screen.queryByRole("link", { name: "Alice Johnson" }),
      ).not.toBeInTheDocument(),
    );
    expect(
      screen.queryByRole("link", { name: "Charlie Brown" }),
    ).not.toBeInTheDocument();
  });

  it("searches users by display name", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/users"],
      config: mockConfigWithUserService,
      children: <UserList />,
    });

    await screen.findByRole("link", { name: "Alice Johnson" });

    await user.click(screen.getByRole("button", { name: "Search" }));
    await user.type(
      await screen.findByRole("textbox", { name: "Display name" }),
      "Alice",
    );
    await user.click(screen.getByRole("button", { name: /find users/i }));

    await waitFor(() =>
      expect(
        screen.queryByRole("link", { name: "Bob Smith" }),
      ).not.toBeInTheDocument(),
    );
    expect(
      screen.getByRole("link", { name: "Alice Johnson" }),
    ).toBeVisible();
  });

  it("searches users by email", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/users"],
      config: mockConfigWithUserService,
      children: <UserList />,
    });

    await screen.findByRole("link", { name: "Alice Johnson" });

    await user.click(screen.getByRole("button", { name: "Search" }));
    await user.type(
      await screen.findByRole("textbox", { name: "Email" }),
      "bob@example.com",
    );
    await user.click(screen.getByRole("button", { name: /find users/i }));

    expect(await screen.findByRole("link", { name: "Bob Smith" })).toBeVisible();
    await waitFor(() =>
      expect(
        screen.queryByRole("link", { name: "Alice Johnson" }),
      ).not.toBeInTheDocument(),
    );
  });

  it("shows no results when identifier is not found", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/users"],
      config: mockConfigWithUserService,
      children: <UserList />,
    });

    await screen.findByRole("link", { name: "Alice Johnson" });

    await user.click(screen.getByRole("button", { name: "Search" }));
    await user.type(
      await screen.findByRole("textbox", { name: "Identifier" }),
      "user-does-not-exist",
    );
    await user.click(screen.getByRole("button", { name: /find users/i }));

    expect(await screen.findByText("No users found.")).toBeVisible();
    expect(
      screen.queryByRole("link", { name: "Alice Johnson" }),
    ).not.toBeInTheDocument();
  });

  it("renders error state when API fails", async () => {
    server.use(
      http.get("http://localhost:9090/api/v1alpha/users", () => {
        return new HttpResponse("Failed to fetch users", { status: 500 });
      }),
    );

    render({
      initialEntries: ["/users"],
      config: mockConfigWithUserService,
      children: <UserList />,
    });

    expect(await screen.findByText(/Failed to fetch users/)).toBeVisible();
  });
});
