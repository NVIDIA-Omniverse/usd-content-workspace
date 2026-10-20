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
import { formatDate, parseStorageTimestamp } from "@/util/date";

describe("parseStorageTimestamp", () => {
  it.each([
    ["UTC designator", "2024-05-15T14:00:00Z", "2024-05-15T14:00:00.000Z"],
    ["colon timezone offset", "2024-05-15T14:00:00+02:00", "2024-05-15T12:00:00.000Z"],
    ["compact timezone offset", "2024-05-15T14:00:00+0200", "2024-05-15T12:00:00.000Z"],
    ["hour timezone offset", "2024-05-15T14:00:00+02", "2024-05-15T12:00:00.000Z"],
    ["bare timestamp", "2024-05-15T14:00:00", "2024-05-15T14:00:00.000Z"],
    ["bare date", "2024-05-15", "2024-05-15T00:00:00.000Z"],
  ])("parses a %s as a UTC instant", (_label, timestamp, expected) => {
    expect(parseStorageTimestamp(timestamp)?.toISOString()).toBe(expected);
  });

  it("returns null for invalid timestamps", () => {
    expect(parseStorageTimestamp("not-a-date")).toBeNull();
  });
});

describe("formatDate", () => {
  it("formats compact timezone offsets the same as colon timezone offsets", () => {
    expect(formatDate("2024-05-15T14:00:00+0200")).toBe(formatDate("2024-05-15T14:00:00+02:00"));
  });

  it("returns an empty string for missing or invalid timestamps", () => {
    expect(formatDate()).toBe("");
    expect(formatDate("not-a-date")).toBe("");
  });
});
