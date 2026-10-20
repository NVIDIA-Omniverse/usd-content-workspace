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

import { fireEvent, render, screen } from "@testing-library/react";
import { vi } from "vitest";
import { MantineProvider } from "@mantine/core";
import { NotificationMessage } from "./NotificationMessage";

describe("NotificationMessage", () => {
  function component() {
    return (
      <MantineProvider>
        <NotificationMessage>
          <div>Test notification content</div>
        </NotificationMessage>
      </MantineProvider>
    );
  }

  it("renders children content", () => {
    render(component());
    expect(screen.getByText("Test notification content")).toBeVisible();
  });

  it("renders cancel button", () => {
    const onCancel = vi.fn();
    render(
      <MantineProvider>
        <NotificationMessage onCancel={onCancel}>
          <div>Test notification content</div>
        </NotificationMessage>
      </MantineProvider>
    );

    const cancelButton = screen.getByRole("button");
    expect(cancelButton).toBeVisible();
  });

  it("calls onCancel when cancel button is clicked", () => {
    const onCancel = vi.fn();
    render(
      <MantineProvider>
        <NotificationMessage onCancel={onCancel}>
          <div>Test notification content</div>
        </NotificationMessage>
      </MantineProvider>
    );

    const cancelButton = screen.getByRole("button");
    fireEvent.click(cancelButton);

    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it("should not display cancel button when onCancel is not provided", () => {
    render(component());

    const cancelButton = screen.queryByRole("button");
    expect(cancelButton).toBeNull();
  });
});
