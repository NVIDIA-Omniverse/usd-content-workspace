/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
import { describe, expect, it } from "vitest";
import { expandResourceHierarchy } from "../diagnostics";

describe("expandResourceHierarchy", () => {
  it("reuses the same type when parent_type is unset", () => {
    const rows = expandResourceHierarchy(
      "https://host.example.com/a/b/c",
      "Folder",
      new Map(),
    );

    expect(rows).toEqual([
      { type: "Folder", id: "https://host.example.com/a/b/" },
      { type: "Folder", id: "https://host.example.com/a/" },
      { type: "Folder", id: "https://host.example.com/" },
    ]);
  });

  it("walks the parent_type chain when it is set", () => {
    const rows = expandResourceHierarchy(
      "https://host.example.com/a/b/c",
      "Object",
      new Map([["Object", "Folder"]]),
    );

    expect(rows).toEqual([
      { type: "Folder", id: "https://host.example.com/a/b/" },
      { type: "Folder", id: "https://host.example.com/a/" },
      { type: "Folder", id: "https://host.example.com/" },
    ]);
  });

  it("resolves multiple levels of parent types", () => {
    const rows = expandResourceHierarchy(
      "https://host.example.com/a/b/c",
      "Object",
      new Map([
        ["Object", "Folder"],
        ["Folder", "Bucket"],
      ]),
    );

    expect(rows).toEqual([
      { type: "Folder", id: "https://host.example.com/a/b/" },
      { type: "Bucket", id: "https://host.example.com/a/" },
      { type: "Bucket", id: "https://host.example.com/" },
    ]);
  });

  it("returns no rows for an invalid resource URL", () => {
    expect(expandResourceHierarchy("not-a-url", "Object", new Map())).toEqual(
      [],
    );
  });
});
