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
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import { render } from "../../util/tests/render";
import { mockConfigWithUserService } from "../../util/tests/config";
import "../../util/tests/msw";
import { PrincipalAutocomplete } from "../PrincipalAutocomplete";

describe("PrincipalAutocomplete", () => {
  it("renders with label and placeholder", () => {
    render({
      initialEntries: ["/"],
      config: mockConfigWithUserService,
      children: <PrincipalAutocomplete />,
    });

    const input = screen.getByRole("textbox", { name: "Principal" });
    expect(input).toBeInTheDocument();
    expect(input).toHaveAttribute(
      "placeholder",
      "Search by name or enter an identifier",
    );
  });

  it("searches users by display name and shows results", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/"],
      config: mockConfigWithUserService,
      children: <PrincipalAutocomplete />,
    });

    const input = screen.getByRole("textbox", { name: "Principal" });
    await user.type(input, "Alice");

    expect(await screen.findByText("Alice Johnson")).toBeInTheDocument();
    expect(screen.getByText("alice@example.com")).toBeInTheDocument();
  });

  it("resolves selected user to user ID by default", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render({
      initialEntries: ["/"],
      config: mockConfigWithUserService,
      children: <PrincipalAutocomplete onChange={onChange} />,
    });

    const input = screen.getByRole("textbox", { name: "Principal" });
    await user.type(input, "Alice");

    const option = await screen.findByText("Alice Johnson");
    await user.click(option);

    expect(onChange).toHaveBeenLastCalledWith("user-123");
  });

  it("resolves selected user to email when idClaim is email", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render({
      initialEntries: ["/"],
      config: mockConfigWithUserService,
      children: <PrincipalAutocomplete idClaim="email" onChange={onChange} />,
    });

    const input = screen.getByRole("textbox", { name: "Principal" });
    await user.type(input, "Bob");

    const option = await screen.findByText("Bob Smith");
    await user.click(option);

    expect(onChange).toHaveBeenLastCalledWith("bob@example.com");
  });

  it("uses typed text as-is when no option is selected", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render({
      initialEntries: ["/"],
      config: mockConfigWithUserService,
      children: <PrincipalAutocomplete onChange={onChange} />,
    });

    const input = screen.getByRole("textbox", { name: "Principal" });
    await user.type(input, "custom-id");

    expect(onChange).toHaveBeenLastCalledWith("custom-id");
  });

  it("shows unscoped option when provided and input is empty", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/"],
      config: mockConfigWithUserService,
      children: <PrincipalAutocomplete unscoped={<span>Unscoped</span>} />,
    });

    const input = screen.getByRole("textbox", { name: "Principal" });
    await user.click(input);

    expect(await screen.findByText("Unscoped")).toBeInTheDocument();
  });

  it("hides unscoped option when text is entered", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/"],
      config: mockConfigWithUserService,
      children: <PrincipalAutocomplete unscoped={<span>Unscoped</span>} />,
    });

    const input = screen.getByRole("textbox", { name: "Principal" });
    await user.type(input, "Alice");

    await screen.findByText("Alice Johnson");
    expect(screen.queryByText("Unscoped")).not.toBeInTheDocument();
  });

  it("sets value to NULL when unscoped option is selected", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render({
      initialEntries: ["/"],
      config: mockConfigWithUserService,
      children: (
        <PrincipalAutocomplete
          unscoped={<span>Unscoped</span>}
          onChange={onChange}
        />
      ),
    });

    const input = screen.getByRole("textbox", { name: "Principal" });
    await user.click(input);

    const unscopedOption = await screen.findByText("Unscoped");
    await user.click(unscopedOption);

    expect(onChange).toHaveBeenLastCalledWith("NULL");
    expect(screen.getByText("Unscoped")).toBeInTheDocument();
  });

  it("does not show 'No users found' when input is empty", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/"],
      config: mockConfigWithUserService,
      children: <PrincipalAutocomplete />,
    });

    const input = screen.getByRole("textbox", { name: "Principal" });
    await user.click(input);

    expect(screen.queryByText("No users found")).not.toBeInTheDocument();
  });

  it("shows 'No users found' when search has no results", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/"],
      config: mockConfigWithUserService,
      children: <PrincipalAutocomplete />,
    });

    const input = screen.getByRole("textbox", { name: "Principal" });
    await user.type(input, "nonexistentuser");

    expect(await screen.findByText("No users found")).toBeInTheDocument();
  });

  it("shows loading spinner while fetching results", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/"],
      config: mockConfigWithUserService,
      children: <PrincipalAutocomplete />,
    });

    const input = screen.getByRole("textbox", { name: "Principal" });
    await user.type(input, "Alice");

    await waitFor(() => {
      expect(screen.getByText("Alice Johnson")).toBeInTheDocument();
    });
  });

  it("shows all matching users for a shared substring", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/"],
      config: mockConfigWithUserService,
      children: <PrincipalAutocomplete />,
    });

    const input = screen.getByRole("textbox", { name: "Principal" });
    await user.type(input, "o");

    expect(await screen.findByText("Alice Johnson")).toBeInTheDocument();
    expect(screen.getByText("Bob Smith")).toBeInTheDocument();
    expect(screen.getByText("Charlie Brown")).toBeInTheDocument();
  });

  it("displays user email in dropdown options", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/"],
      config: mockConfigWithUserService,
      children: <PrincipalAutocomplete />,
    });

    const input = screen.getByRole("textbox", { name: "Principal" });
    await user.type(input, "Bob");

    await screen.findByText("Bob Smith");
    expect(screen.getByText("bob@example.com")).toBeInTheDocument();
  });

  it("calls onResolvedChange with true when user is selected", async () => {
    const user = userEvent.setup();
    const onResolvedChange = vi.fn();
    render({
      initialEntries: ["/"],
      config: mockConfigWithUserService,
      children: (
        <PrincipalAutocomplete onResolvedChange={onResolvedChange} />
      ),
    });

    const input = screen.getByRole("textbox", { name: "Principal" });
    await user.type(input, "Alice");

    const option = await screen.findByText("Alice Johnson");
    await user.click(option);

    expect(onResolvedChange).toHaveBeenLastCalledWith(true);
  });

  it("calls onResolvedChange with false when input is modified", async () => {
    const user = userEvent.setup();
    const onResolvedChange = vi.fn();
    render({
      initialEntries: ["/"],
      config: mockConfigWithUserService,
      children: (
        <PrincipalAutocomplete onResolvedChange={onResolvedChange} />
      ),
    });

    const input = screen.getByRole("textbox", { name: "Principal" });
    await user.type(input, "test");

    expect(onResolvedChange).toHaveBeenLastCalledWith(false);
  });

  it("clears value when clear button is clicked", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render({
      initialEntries: ["/"],
      config: mockConfigWithUserService,
      children: <PrincipalAutocomplete value="user-123" onChange={onChange} />,
    });

    const clearButton = screen.getByRole("button", { name: "Clear principal" });
    await user.click(clearButton);

    expect(onChange).toHaveBeenLastCalledWith("");
  });

  it("falls back to FilterInput when user service is not configured and unscoped is provided", () => {
    render({
      initialEntries: ["/"],
      children: <PrincipalAutocomplete unscoped={<span>Unscoped</span>} />,
    });

    const input = screen.getByRole("textbox", { name: "Principal" });
    expect(input).toBeInTheDocument();
  });

  it("falls back to plain TextInput when user service is not configured", () => {
    render({
      initialEntries: ["/"],
      children: <PrincipalAutocomplete />,
    });

    const input = screen.getByRole("textbox", { name: "Principal" });
    expect(input).toBeInTheDocument();
  });

  it("resolves initial value to display name when value is a known user ID", async () => {
    render({
      initialEntries: ["/"],
      config: mockConfigWithUserService,
      children: <PrincipalAutocomplete value="user-123" />,
    });

    const input = screen.getByRole("textbox", {
      name: "Principal",
    }) as HTMLInputElement;

    expect(input.value).toBe("user-123");

    await waitFor(() => {
      expect(input.value).toBe("Alice Johnson");
    });
  });

  it("resolves initial value to display name when idClaim is email", async () => {
    render({
      initialEntries: ["/"],
      config: mockConfigWithUserService,
      children: (
        <PrincipalAutocomplete idClaim="email" value="bob@example.com" />
      ),
    });

    const input = screen.getByRole("textbox", {
      name: "Principal",
    }) as HTMLInputElement;

    await waitFor(() => {
      expect(input.value).toBe("Bob Smith");
    });
  });

  it("calls onResolvedChange with true when initial value resolves to a known user", async () => {
    const onResolvedChange = vi.fn();
    render({
      initialEntries: ["/"],
      config: mockConfigWithUserService,
      children: (
        <PrincipalAutocomplete
          value="user-123"
          onResolvedChange={onResolvedChange}
        />
      ),
    });

    await waitFor(() => {
      expect(onResolvedChange).toHaveBeenCalledWith(true);
    });
  });

  it("keeps initial value as-is when it cannot be resolved to a known user", async () => {
    const onResolvedChange = vi.fn();
    render({
      initialEntries: ["/"],
      config: mockConfigWithUserService,
      children: (
        <PrincipalAutocomplete
          value="service-account@example.com"
          onResolvedChange={onResolvedChange}
        />
      ),
    });

    const input = screen.getByRole("textbox", {
      name: "Principal",
    }) as HTMLInputElement;

    await waitFor(() => {
      expect(onResolvedChange).not.toHaveBeenCalledWith(true);
    });
    expect(input.value).toBe("service-account@example.com");
  });

  it("does not override user input when initial resolution arrives after typing", async () => {
    const user = userEvent.setup();
    render({
      initialEntries: ["/"],
      config: mockConfigWithUserService,
      children: <PrincipalAutocomplete value="user-123" />,
    });

    const input = screen.getByRole("textbox", {
      name: "Principal",
    }) as HTMLInputElement;

    await user.clear(input);
    await user.type(input, "custom");

    await waitFor(() => {
      expect(input.value).toBe("custom");
    });
    expect(input.value).not.toBe("Alice Johnson");
  });

  it("defaults the type selector to User", () => {
    render({
      initialEntries: ["/"],
      config: mockConfigWithUserService,
      children: <PrincipalAutocomplete />,
    });

    expect(screen.getByLabelText("Principal type: User")).toBeInTheDocument();
  });

  it("switches to group search and lists groups", async () => {
    const user = userEvent.setup();
    const onPrincipalTypeChange = vi.fn();

    function Wrapper() {
      const [type, setType] = useState<"user" | "group">("user");
      return (
        <PrincipalAutocomplete
          principalType={type}
          onPrincipalTypeChange={(next) => {
            onPrincipalTypeChange(next);
            setType(next);
          }}
        />
      );
    }

    render({
      initialEntries: ["/"],
      config: mockConfigWithUserService,
      children: <Wrapper />,
    });

    await user.click(screen.getByLabelText("Principal type: User"));
    await user.click(await screen.findByRole("menuitem", { name: "Group" }));

    expect(onPrincipalTypeChange).toHaveBeenLastCalledWith("group");

    const input = screen.getByRole("textbox", { name: "Principal" });
    await user.type(input, "Platform");

    expect(await screen.findByText("Platform Team")).toBeInTheDocument();
  });

  it("resolves a selected group to its id", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();

    render({
      initialEntries: ["/"],
      config: mockConfigWithUserService,
      children: (
        <PrincipalAutocomplete principalType="group" onChange={onChange} />
      ),
    });

    const input = screen.getByRole("textbox", { name: "Principal" });
    await user.type(input, "Platform");

    const option = await screen.findByText("Platform Team");
    await user.click(option);

    expect(onChange).toHaveBeenLastCalledWith("group-123");
  });

  it("renders the type selector read-only when locked", () => {
    render({
      initialEntries: ["/"],
      config: mockConfigWithUserService,
      children: <PrincipalAutocomplete principalType="user" lockPrincipalType />,
    });

    expect(screen.getByLabelText("Principal type: User")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Principal type: User" }),
    ).not.toBeInTheDocument();
  });

  it("does not show dropdown when NULL is entered", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render({
      initialEntries: ["/"],
      config: mockConfigWithUserService,
      children: (
        <PrincipalAutocomplete
          unscoped={<span>Unscoped</span>}
          onChange={onChange}
        />
      ),
    });

    const input = screen.getByRole("textbox", { name: "Principal" });
    await user.click(input);

    const unscopedOption = await screen.findByText("Unscoped");
    await user.click(unscopedOption);

    expect(screen.getByText("Unscoped")).toBeInTheDocument();
    expect(screen.queryByText("No users found")).not.toBeInTheDocument();
    expect(screen.queryByText("Loading users...")).not.toBeInTheDocument();
  });
});
