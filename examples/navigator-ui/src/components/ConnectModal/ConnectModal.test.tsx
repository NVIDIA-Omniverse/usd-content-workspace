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
import { renderModal, TestModalProps } from "@/test-utils/render";

describe("ConnectModal", () => {
  const onAddConnection = vi.fn();
  const onClose = vi.fn();

  const modal: TestModalProps<"connect"> = {
    modal: "connect",
    title: "Connect",
    innerProps: {
      onAddConnection,
    },
    onClose,
  };

  beforeEach(() => {
    onAddConnection.mockClear();
    onClose.mockClear();
  });

  it("renders correctly", async () => {
    await renderModal(modal);

    const title = screen.getByRole("heading", { level: 2 });
    expect(title).toBeVisible();
    expect(title).toHaveTextContent("Connect");

    const input = screen.getByLabelText("Discovery Hostname");
    expect(input).toBeVisible();

    // focus is set via the useFocusTrap hook
    await waitFor(() => expect(input).toHaveFocus());
  });

  it("calls onAddConnection when form is submitted with valid endpoint", async () => {
    await renderModal(modal);

    const input = screen.getByRole("textbox", { name: "Discovery Hostname" });
    const submitButton = screen.getByRole("button", { name: /connect$/i });

    await userEvent.type(input, "https://example.com");
    await userEvent.click(submitButton);

    expect(onAddConnection).toHaveBeenCalledOnce();
    expect(onAddConnection).toHaveBeenCalledWith("https://example.com");
  });

  it("shows an error when the added endpoint is invalid", async () => {
    onAddConnection.mockImplementationOnce(() => {
      throw new Error("Test error");
    });

    await renderModal(modal);

    const input = screen.getByRole("textbox", { name: "Discovery Hostname" });
    const submitButton = screen.getByRole("button", { name: /connect$/i });

    await userEvent.type(input, "none");
    await userEvent.click(submitButton);

    expect(onAddConnection).toHaveBeenCalledWith("none");

    const error = await screen.findByText(/Test error/);
    expect(error).toBeVisible();
  });

  it("calls close function when modal is closed", async () => {
    await renderModal(modal);

    const closeButton = screen.getByRole("button", { name: "" });

    await userEvent.click(closeButton);
    expect(onClose).toHaveBeenCalledOnce();
  });
});
