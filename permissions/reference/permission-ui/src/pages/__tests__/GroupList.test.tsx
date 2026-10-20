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
import { GroupList } from "../GroupList";
import { server } from "../../util/tests/msw";
import { render } from "../../util/tests/render";
import { mockConfigWithUserService } from "../../util/tests/config";

describe("GroupList", () => {
  it("renders loading state initially", () => {
    render({
      initialEntries: ["/groups"],
      config: mockConfigWithUserService,
      children: <GroupList />,
    });

    expect(screen.getByText("Groups")).toBeVisible();
    const loader = document.querySelector(".mantine-Loader-root");
    expect(loader).toBeInTheDocument();
  });

  it("renders groups list after loading", async () => {
    render({
      initialEntries: ["/groups"],
      config: mockConfigWithUserService,
      children: <GroupList />,
    });

    expect(
      await screen.findByRole("link", { name: "Platform Team" }),
    ).toBeVisible();
    expect(screen.getByRole("link", { name: "Security Team" })).toBeVisible();
    expect(screen.getByRole("link", { name: "Data Team" })).toBeVisible();
  });

  it("searches groups by identifier", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/groups"],
      config: mockConfigWithUserService,
      children: <GroupList />,
    });

    await screen.findByRole("link", { name: "Platform Team" });

    await user.click(screen.getByRole("button", { name: "Search" }));
    await user.type(
      await screen.findByRole("textbox", { name: "Identifier" }),
      "group-456",
    );
    await user.click(screen.getByRole("button", { name: /find groups/i }));

    expect(
      await screen.findByRole("link", { name: "Security Team" }),
    ).toBeVisible();
    await waitFor(() =>
      expect(
        screen.queryByRole("link", { name: "Platform Team" }),
      ).not.toBeInTheDocument(),
    );
    expect(
      screen.queryByRole("link", { name: "Data Team" }),
    ).not.toBeInTheDocument();
  });

  it("searches groups by display name", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/groups"],
      config: mockConfigWithUserService,
      children: <GroupList />,
    });

    await screen.findByRole("link", { name: "Platform Team" });

    await user.click(screen.getByRole("button", { name: "Search" }));
    await user.type(
      await screen.findByRole("textbox", { name: "Display name" }),
      "Security",
    );
    await user.click(screen.getByRole("button", { name: /find groups/i }));

    expect(
      await screen.findByRole("link", { name: "Security Team" }),
    ).toBeVisible();
    await waitFor(() =>
      expect(
        screen.queryByRole("link", { name: "Platform Team" }),
      ).not.toBeInTheDocument(),
    );
  });

  it("shows no results when identifier is not found", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/groups"],
      config: mockConfigWithUserService,
      children: <GroupList />,
    });

    await screen.findByRole("link", { name: "Platform Team" });

    await user.click(screen.getByRole("button", { name: "Search" }));
    await user.type(
      await screen.findByRole("textbox", { name: "Identifier" }),
      "group-does-not-exist",
    );
    await user.click(screen.getByRole("button", { name: /find groups/i }));

    expect(await screen.findByText("No groups found.")).toBeVisible();
    expect(
      screen.queryByRole("link", { name: "Platform Team" }),
    ).not.toBeInTheDocument();
  });

  it("renders error state when API fails", async () => {
    server.use(
      http.get("http://localhost:9090/api/v1alpha/groups", () => {
        return new HttpResponse("Failed to fetch groups", { status: 500 });
      }),
    );

    render({
      initialEntries: ["/groups"],
      config: mockConfigWithUserService,
      children: <GroupList />,
    });

    expect(await screen.findByText(/Failed to fetch groups/)).toBeVisible();
  });
});
