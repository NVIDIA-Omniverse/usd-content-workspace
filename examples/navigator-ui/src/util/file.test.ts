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

import { describe, expect, it } from "vitest";
import { formatBytes } from "@/util/file";

describe("formatBytes function", () => {
  it("formats different byte sizes correctly", () => {
    expect(formatBytes(0)).toBe("0 Bytes");
    expect(formatBytes(100)).toBe("100 Bytes");
    expect(formatBytes(1024)).toBe("1 KiB");
    expect(formatBytes(1536)).toBe("1.5 KiB");
    expect(formatBytes(1048576)).toBe("1 MiB");
    expect(formatBytes(1073741824)).toBe("1 GiB");
  });

  it("respects the decimals parameter", () => {
    expect(formatBytes(1500, 0)).toBe("1 KiB");
    expect(formatBytes(1500, 1)).toBe("1.5 KiB");
    expect(formatBytes(1500, 3)).toBe("1.465 KiB");
  });
});
