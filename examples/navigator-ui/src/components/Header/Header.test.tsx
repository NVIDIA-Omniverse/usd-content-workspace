// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

import { act, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { beforeEach } from "vitest";
import { Header } from "@/components/Header/Header";
import { constructPathNavLink } from "@/hooks/usePathNavigation";
import { useAppStore } from "@/state/appStore";
import { render } from "@/test-utils";
import { testConnection } from "@/test-utils/msw";

vi.mock("@/hooks/useAppConfig", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/hooks/useAppConfig")>();
  return {
    ...actual,
    useAppConfig: () => ({ defaultDiscovery: "", references: [] }),
  };
});

describe("Header", () => {
  const component = () => (
    <MemoryRouter>
      <Header />
    </MemoryRouter>
  );

  beforeEach(() => {
    act(() => {
      useAppStore.setState(useAppStore.getInitialState());
    });
  });

  it("displays breadcrumbs", async () => {
    render(component());

    act(() => {
      useAppStore.setState({
        connections: {
          [testConnection.endpoint]: testConnection,
        },
        endpoints: new Set([testConnection.endpoint]),
        activeConnection: testConnection.endpoint,
      });
    });

    const connection: HTMLAnchorElement = await screen.findByRole("link", { name: "test-endpoint" });
    expect(connection).toBeVisible();
    expect(connection.href).toBe(window.location.origin + constructPathNavLink(testConnection.endpoint));

    act(() => {
      useAppStore.setState({
        connections: {
          [testConnection.endpoint]: {
            ...testConnection,
            currentFolder: "http://test-server/bucket/folder/subfolder",
          },
        },
      });
    });

    const bucket: HTMLAnchorElement = await screen.findByRole("link", { name: "bucket" });
    expect(bucket).toBeVisible();
    expect(bucket.href).toBe(
      window.location.origin + constructPathNavLink(testConnection.endpoint, "http://test-server/bucket")
    );

    const folder: HTMLAnchorElement = await screen.findByRole("link", { name: "folder" });
    expect(folder).toBeVisible();
    expect(folder.href).toBe(
      window.location.origin + constructPathNavLink(testConnection.endpoint, "http://test-server/bucket/folder")
    );

    const subfolder: HTMLAnchorElement = await screen.findByRole("link", { name: "subfolder" });
    expect(subfolder).toBeVisible();
    expect(subfolder.href).toBe(
      window.location.origin +
        constructPathNavLink(testConnection.endpoint, "http://test-server/bucket/folder/subfolder")
    );
  });
});
