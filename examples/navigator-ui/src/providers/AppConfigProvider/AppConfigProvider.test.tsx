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

import { AppConfig, AppConfigError } from "@/services/appConfigService";
import { screen } from "@/test-utils";
import { render } from "@/test-utils/render";
import { AppConfigProvider } from "./AppConfigProvider";

const useAppConfig = vi.fn<() => AppConfig>();

vi.mock("@/hooks/useAppConfig", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/hooks/useAppConfig")>();
  return {
    ...actual,
    useAppConfig: () => useAppConfig(),
  };
});

describe("AppConfigProvider", () => {
  it("renders children when the config loads successfully", () => {
    useAppConfig.mockReturnValue({ defaultDiscovery: "", references: [], requestTimeout: 30000, batchSize: 25 });

    render(
      <AppConfigProvider>
        <div>app content</div>
      </AppConfigProvider>
    );

    expect(screen.getByText("app content")).toBeVisible();
  });

  it("displays the validation error and offending fields when the config is invalid", () => {
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => undefined);
    useAppConfig.mockImplementation(() => {
      throw new AppConfigError("Configuration file (config.json) failed validation.", ["references.0.url: Required"]);
    });

    try {
      render(
        <AppConfigProvider>
          <div>app content</div>
        </AppConfigProvider>
      );

      expect(screen.getByText("Invalid application configuration")).toBeVisible();
      expect(screen.getByText("Configuration file (config.json) failed validation.")).toBeVisible();
      expect(screen.getByText("references.0.url: Required")).toBeVisible();
      expect(screen.queryByText("app content")).not.toBeInTheDocument();
    } finally {
      consoleError.mockRestore();
    }
  });
});
