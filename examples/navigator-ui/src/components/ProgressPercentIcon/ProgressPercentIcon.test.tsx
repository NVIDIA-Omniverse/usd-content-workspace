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

import { render, screen } from "@testing-library/react";
import { MantineProvider } from "@mantine/core";
import { ProgressPercentIcon } from "./ProgressPercentIcon";

describe("ProgressPercentIcon", () => {
  function component(percent: number) {
    return (
      <MantineProvider>
        <ProgressPercentIcon percent={percent} />
      </MantineProvider>
    );
  }

  it("renders with correct percentage", () => {
    render(component(75));
    expect(screen.getByText("75%")).toBeVisible();
  });

  it("rounds decimal percentages to whole numbers", () => {
    render(component(75.7));
    expect(screen.getByText("76%")).toBeVisible();
  });

  it("handles zero percentage", () => {
    render(component(0));
    expect(screen.getByText("0%")).toBeVisible();
  });

  it("handles 100 percentage", () => {
    render(component(100));
    expect(screen.getByText("100%")).toBeVisible();
  });

  it("renders RingProgress component", () => {
    const { container } = render(component(50));
    expect(container.querySelector(".mantine-RingProgress-root")).toBeVisible();
  });
});
