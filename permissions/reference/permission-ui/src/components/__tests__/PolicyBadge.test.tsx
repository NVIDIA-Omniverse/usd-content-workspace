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
import { screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { render } from "../../util/tests/render";
import { mockConfigWithUserService } from "../../util/tests/config";
import "../../util/tests/msw";
import { PolicyBadge } from "../PolicyBadge";
import type { Policy } from "../../state/policies";

describe("PolicyBadge.Principal", () => {
  const policyWithPrincipal: Policy = {
    id: 1,
    policy: 'permit(principal == Principal::"user-123", action, resource);',
    principal: { sub: "user-123", info: null },
    action: null,
    resource: null,
  };

  const policyWithUnknownPrincipal: Policy = {
    id: 2,
    policy: 'permit(principal == Principal::"unknown-id", action, resource);',
    principal: { sub: "unknown-id", info: null },
    action: null,
    resource: null,
  };

  const unscopedPolicy: Policy = {
    id: 3,
    policy: "permit(principal, action, resource);",
    principal: null,
    action: null,
    resource: null,
  };

  it("shows 'Unscoped' badge when policy has no principal", () => {
    render({
      initialEntries: ["/"],
      config: mockConfigWithUserService,
      children: <PolicyBadge.Principal policy={unscopedPolicy} />,
    });

    expect(screen.getByText("Unscoped")).toBeVisible();
  });

  it("resolves principal sub to display name when user service is available", async () => {
    render({
      initialEntries: ["/"],
      config: mockConfigWithUserService,
      children: <PolicyBadge.Principal policy={policyWithPrincipal} />,
    });

    expect(await screen.findByText("Alice Johnson")).toBeVisible();
  });

  it("shows raw sub when user cannot be found", async () => {
    render({
      initialEntries: ["/"],
      config: mockConfigWithUserService,
      children: (
        <PolicyBadge.Principal policy={policyWithUnknownPrincipal} />
      ),
    });

    expect(await screen.findByText("unknown-id")).toBeVisible();
  });

  it("shows raw sub when user service is not configured", () => {
    render({
      initialEntries: ["/"],
      children: <PolicyBadge.Principal policy={policyWithPrincipal} />,
    });

    expect(screen.getByText("user-123")).toBeVisible();
  });

  it("shows 'Unscoped' badge when no policy is provided", () => {
    render({
      initialEntries: ["/"],
      children: <PolicyBadge.Principal />,
    });

    expect(screen.getByText("Unscoped")).toBeVisible();
  });

  const groupPolicy: Policy = {
    id: 4,
    policy: 'permit(principal in Group::"group-123", action, resource);',
    principal: null,
    group: { id: "group-123" },
    action: null,
    resource: null,
  };

  const unknownGroupPolicy: Policy = {
    id: 5,
    policy: 'permit(principal in Group::"unknown-group", action, resource);',
    principal: null,
    group: { id: "unknown-group" },
    action: null,
    resource: null,
  };

  it("resolves group id to display name when user service is available", async () => {
    render({
      initialEntries: ["/"],
      config: mockConfigWithUserService,
      children: <PolicyBadge.Principal policy={groupPolicy} />,
    });

    expect(await screen.findByText("Platform Team")).toBeVisible();
  });

  it("shows raw group id when group cannot be found", async () => {
    render({
      initialEntries: ["/"],
      config: mockConfigWithUserService,
      children: <PolicyBadge.Principal policy={unknownGroupPolicy} />,
    });

    expect(await screen.findByText("unknown-group")).toBeVisible();
  });

  it("shows raw group id when user service is not configured", () => {
    render({
      initialEntries: ["/"],
      children: <PolicyBadge.Principal policy={groupPolicy} />,
    });

    expect(screen.getByText("group-123")).toBeVisible();
  });
});
