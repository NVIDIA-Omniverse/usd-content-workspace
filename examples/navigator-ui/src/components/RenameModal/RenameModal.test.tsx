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
import { beforeEach, describe, expect, it, vi } from "vitest";
import { modals } from "@mantine/modals";
import { userEvent } from "@/test-utils";
import { renderModal, TestModalProps } from "@/test-utils/render";

describe("RenameModal", () => {
  const onSubmit = vi.fn();
  const onClose = vi.fn();

  const modal: TestModalProps<"rename"> = {
    modal: "rename",
    title: "Rename",
    innerProps: {
      initial: "Test Name",
      onSubmit,
    },
    onClose,
  };

  beforeEach(() => {
    onSubmit.mockClear();
    onClose.mockClear();
  });

  it("renders correctly when opened", async () => {
    await renderModal(modal);

    expect(screen.getByRole("heading", { name: "Rename" })).toBeVisible();
    expect(screen.getByLabelText(/New name/i)).toBeVisible();
    expect(screen.getByLabelText(/New name/i)).toHaveValue("Test Name");
    expect(screen.getByRole("button", { name: /Rename/i })).toBeVisible();
    expect(screen.getByRole("button", { name: /Cancel/i })).toBeVisible();
  });

  it("updates input value when initial prop changes", async () => {
    const modalId = await renderModal(modal);
    expect(screen.getByLabelText(/New name/i)).toHaveValue("Test Name");

    act(() => {
      modals.updateContextModal({
        modalId,
        innerProps: {
          initial: "Updated Name",
        },
      });
    });
    expect(screen.getByLabelText(/New name/i)).toHaveValue("Updated Name");
  });

  it("shows validation error when submitting with empty name", async () => {
    await renderModal(modal);

    const input = screen.getByLabelText(/New name/i);
    await userEvent.clear(input);

    const submitButton = screen.getByRole("button", { name: /Rename/i });
    await userEvent.click(submitButton);

    expect(screen.getByText("Name must not be blank.")).toBeVisible();
    expect(onSubmit).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
  });

  it("shows validation error when submitting name with invalid characters", async () => {
    await renderModal(modal);

    const input = screen.getByLabelText(/New name/i);
    await userEvent.type(input, "test?.png");

    const submitButton = screen.getByRole("button", { name: /Rename/i });
    await userEvent.click(submitButton);

    expect(
      screen.getByText("Name must not contain system characters such as slashes, colons, question marks and quotes.")
    ).toBeVisible();

    expect(onSubmit).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
  });

  it("calls onSubmit with new name when form is submitted with valid data", async () => {
    await renderModal(modal);

    const input = screen.getByLabelText(/New name/i);
    await userEvent.clear(input);
    await userEvent.type(input, "New Test Name");

    const submitButton = screen.getByRole("button", { name: /Rename/i });
    await userEvent.click(submitButton);

    expect(onSubmit).toHaveBeenCalledWith("New Test Name");
    expect(onClose).toHaveBeenCalled();
  });

  it("does not call onSubmit when name is unchanged", async () => {
    await renderModal(modal);

    const submitButton = screen.getByRole("button", { name: /Rename/i });
    await userEvent.click(submitButton);

    expect(onSubmit).not.toHaveBeenCalled();
    expect(onClose).toHaveBeenCalled();
  });

  it("calls onClose when Cancel button is clicked", async () => {
    await renderModal(modal);

    const cancelButton = screen.getByRole("button", { name: /Cancel/i });
    await userEvent.click(cancelButton);

    expect(onSubmit).not.toHaveBeenCalled();
    expect(onClose).toHaveBeenCalled();
  });
});
