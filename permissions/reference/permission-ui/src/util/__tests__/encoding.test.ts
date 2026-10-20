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
import { encodeResourceId } from "../encoding";

describe("encodeResourceId", () => {
  it("returns non-URL input unchanged", () => {
    expect(encodeResourceId("doc-123")).toBe("doc-123");
    expect(encodeResourceId("")).toBe("");
  });

  it("percent-encodes raw spaces in URL path segments exactly once", () => {
    expect(encodeResourceId("http://test/abc def g/")).toBe(
      "http://test/abc%20def%20g/",
    );
  });

  it("leaves an already-encoded URL unchanged", () => {
    const canonical =
      "https://example.blob.core.windows.net/Rohan%20-Service-permission-test/";
    expect(encodeResourceId(canonical)).toBe(canonical);
  });

  it("is idempotent (regression: `%20` must not escalate to `%2520` when re-encoded)", () => {
    const raw = "http://test/abc def g/";
    const once = encodeResourceId(raw);
    const twice = encodeResourceId(once);
    expect(twice).toBe(once);
    expect(once).not.toContain("%2520");
  });

  it("preserves literal `%2520` instead of decoding back to a space", () => {
    // `%25` decodes to `%`, so `%2520` decodes to `%20` and re-encodes to
    // `%2520`. The function never escalates an already-double-encoded value.
    const doubled =
      "https://example.com/Rohan%2520-Service-permission-test/";
    expect(encodeResourceId(doubled)).toBe(doubled);
  });

  it("passes through segments with malformed percent sequences", () => {
    // `%` not followed by two hex digits trips decodeURIComponent; the
    // segment must be left as-is rather than throwing.
    expect(encodeResourceId("http://test/abc%2/")).toBe("http://test/abc%2/");
  });
});
