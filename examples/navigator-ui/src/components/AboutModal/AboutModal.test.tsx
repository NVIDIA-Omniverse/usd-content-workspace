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

import { screen, userEvent } from "@/test-utils";
import { renderModal, TestModalProps } from "@/test-utils/render";

const onClose = vi.fn();

const modal: TestModalProps<"about"> = {
  modal: "about",
  innerProps: {},
  onClose,
};

describe("AboutModal", () => {
  it("displays the application name", async () => {
    await renderModal(modal);

    expect(screen.getByText("Storage Navigator")).toBeVisible();
  });

  it("displays the version number", async () => {
    await renderModal(modal);

    expect(screen.getByText(__APP_VERSION__)).toBeVisible();
  });

  it("displays the logo", async () => {
    await renderModal(modal);

    const logo = screen.getByRole("img");
    expect(logo).toBeVisible();
  });

  it("displays a link to third-party licenses", async () => {
    await renderModal(modal);

    const link: HTMLAnchorElement = screen.getByRole("link", { name: "Third-party licenses" });
    expect(link).toBeVisible();
    expect(link.href).toContain("/ATTRIBUTIONS.md");
    expect(link.target).toBe("_blank");
  });

  it("displays a close button", async () => {
    await renderModal(modal);

    expect(screen.getByRole("button", { name: "Close" })).toBeVisible();
  });

  it("closes the modal when the close button is clicked", async () => {
    await renderModal(modal);

    await userEvent.click(screen.getByRole("button", { name: "Close" }));

    expect(onClose).toHaveBeenCalledOnce();
  });
});
