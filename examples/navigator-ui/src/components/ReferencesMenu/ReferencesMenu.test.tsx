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

import type { AppConfig } from "@/services/appConfigService";
import { screen, userEvent, waitFor } from "@/test-utils";
import { render } from "@/test-utils/render";
import { ReferencesMenu } from "./ReferencesMenu";

const useAppConfig = vi.fn<() => AppConfig>();

vi.mock("@/hooks/useAppConfig", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/hooks/useAppConfig")>();
  return {
    ...actual,
    useAppConfig: () => useAppConfig(),
  };
});

function setReferences(references: AppConfig["references"]) {
  useAppConfig.mockReturnValue({ defaultDiscovery: "", references, requestTimeout: 30000, batchSize: 25 });
}

describe("ReferencesMenu", () => {
  it("renders nothing when no references are configured", () => {
    setReferences([]);

    render(<ReferencesMenu />);

    expect(screen.queryByRole("button", { name: "Reference links" })).not.toBeInTheDocument();
  });

  it("renders the trigger button when references exist", () => {
    setReferences([{ label: "Docs", url: "https://docs.example/", icon: "book" }]);

    render(<ReferencesMenu />);

    expect(screen.getByRole("button", { name: "Reference links" })).toBeVisible();
  });

  it("renders each link with its url and a new-tab target on open", async () => {
    setReferences([
      { label: "Docs", url: "https://docs.example/", icon: "book" },
      { label: "Support", url: "https://support.example/" },
    ]);

    render(<ReferencesMenu />);
    await userEvent.click(screen.getByRole("button", { name: "Reference links" }));

    const docs = await screen.findByRole("menuitem", { name: "Docs" });
    expect(docs).toHaveAttribute("href", "https://docs.example/");
    expect(docs).toHaveAttribute("target", "_blank");
    expect(docs).toHaveAttribute("rel", "noopener noreferrer");

    const support = screen.getByRole("menuitem", { name: "Support" });
    expect(support).toHaveAttribute("href", "https://support.example/");
  });

  it("renders the configured icon glyph and omits it when no icon is set", async () => {
    setReferences([
      { label: "Docs", url: "https://docs.example/", icon: "book" },
      { label: "Support", url: "https://support.example/" },
    ]);

    render(<ReferencesMenu />);
    await userEvent.click(screen.getByRole("button", { name: "Reference links" }));

    const docs = await screen.findByRole("menuitem", { name: "Docs" });
    expect(docs.querySelector("i.ti.ti-book")).toBeInTheDocument();

    const support = screen.getByRole("menuitem", { name: "Support" });
    await waitFor(() => expect(support.querySelector("i[class*='ti-']")).not.toBeInTheDocument());
  });
});
