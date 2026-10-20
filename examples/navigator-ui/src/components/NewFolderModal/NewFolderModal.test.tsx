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

describe("NewFolderModal", () => {
  const onCreateFolder = vi.fn();
  const onClose = vi.fn();

  const modal: TestModalProps<"newFolder"> = {
    modal: "newFolder",
    title: "New Folder",
    innerProps: {
      onCreateFolder,
    },
    onClose,
  };

  beforeEach(() => {
    onCreateFolder.mockClear();
    onClose.mockClear();
  });

  it("creates a new folder", async () => {
    await renderModal(modal);

    const nameInput = screen.getByRole("textbox", { name: "Folder name" });
    await userEvent.type(nameInput, "Test Folder");

    const createButton = screen.getByRole("button", { name: "Create" });
    await userEvent.click(createButton);

    expect(onCreateFolder).toBeCalledWith("Test Folder");
    expect(onClose).toBeCalled();
  });

  it("displays an error if name is blank", async () => {
    await renderModal(modal);

    const nameInput = screen.getByRole("textbox", { name: "Folder name" });
    await userEvent.type(nameInput, "    ");

    const createButton = screen.getByRole("button", { name: "Create" });
    await userEvent.click(createButton);

    const error = await screen.findByText("Folder name must not be blank.");
    expect(error).toBeVisible();
  });

  it("displays an error if name has invalid characters", async () => {
    await renderModal(modal);

    const nameInput = screen.getByRole("textbox", { name: "Folder name" });
    await userEvent.type(nameInput, "test/1/?/3");

    const createButton = screen.getByRole("button", { name: "Create" });
    await userEvent.click(createButton);

    const error = await screen.findByText(
      "Folder name must not contain system characters such as slashes, colons, question marks and quotes."
    );
    expect(error).toBeVisible();
  });

  it("displays an error if fails to create a folder", async () => {
    const testError = new Error("Test Error");
    onCreateFolder.mockImplementationOnce(() => {
      throw testError;
    });

    await renderModal(modal);

    const nameInput = screen.getByRole("textbox", { name: "Folder name" });
    await userEvent.type(nameInput, "Test Folder");

    const createButton = screen.getByRole("button", { name: "Create" });
    await userEvent.click(createButton);

    const error = await screen.findByText(/Test Error/);
    expect(error).toBeVisible();
  });
});
