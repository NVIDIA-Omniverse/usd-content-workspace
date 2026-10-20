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

import { screen, userEvent, waitFor } from "@/test-utils";
import { render } from "@/test-utils/render";
import { AppMenu } from "./AppMenu";

vi.mock("@mantine/modals", async (importOriginal) => {
  const original = await importOriginal<typeof import("@mantine/modals")>();
  return {
    ...original,
    modals: {
      ...original.modals,
      openContextModal: vi.fn(),
    },
  };
});

describe("AppMenu", () => {
  it("renders the hamburger menu button", () => {
    render(<AppMenu />);

    expect(screen.getByRole("button", { name: "Application menu" })).toBeVisible();
  });

  it("expands menu on click", async () => {
    render(<AppMenu />);

    const button = screen.getByRole("button", { name: "Application menu" });
    await userEvent.click(button);

    await waitFor(() => {
      expect(button).toHaveAttribute("aria-expanded", "true");
    });
  });

  it("opens the About modal when About is clicked", async () => {
    const { modals } = await import("@mantine/modals");

    render(<AppMenu />);

    await userEvent.click(screen.getByRole("button", { name: "Application menu" }));
    await userEvent.click(await screen.findByText("About"));

    expect(modals.openContextModal).toHaveBeenCalledWith(expect.objectContaining({ modal: "about" }));
  });
});
