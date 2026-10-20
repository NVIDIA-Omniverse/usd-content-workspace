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
import { ensureFolderPath, getPathName, join } from "./url";

describe("ensureFolderPath", () => {
  it("adds trailing slash if missing", () => {
    expect(ensureFolderPath("omniverse://host/path")).toBe(
      "omniverse://host/path/",
    );
  });
  it("leaves trailing slash alone", () => {
    expect(ensureFolderPath("omniverse://host/path/")).toBe(
      "omniverse://host/path/",
    );
  });
});

describe("getPathName", () => {
  it("returns last segment of a folder path", () => {
    expect(getPathName("omniverse://host/Library/")).toBe("Library");
  });
  it("returns filename from file path", () => {
    expect(getPathName("omniverse://host/Library/scene.usd")).toBe("scene.usd");
  });
});

describe("join", () => {
  it("joins path segments", () => {
    expect(join("https://host", "/api/v1", "/")).toBe("https://host/api/v1/");
  });
});
