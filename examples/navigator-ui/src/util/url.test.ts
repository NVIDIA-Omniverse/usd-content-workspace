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
import { decodePathName, getFileExtension, getParents, getPathName, join } from "./url";

describe("decodePathName", () => {
  it("decodes URI-encoded path segments", () => {
    expect(decodePathName("file%20name.txt")).toBe("file name.txt");
    expect(decodePathName("my%2Ffolder")).toBe("my/folder");
    expect(decodePathName("100%25")).toBe("100%");
  });

  it("returns plain strings unchanged", () => {
    expect(decodePathName("file.txt")).toBe("file.txt");
    expect(decodePathName("folder")).toBe("folder");
    expect(decodePathName("")).toBe("");
  });

  it("returns the original string when decoding throws", () => {
    expect(decodePathName("bad%")).toBe("bad%");
    expect(decodePathName("bad%2")).toBe("bad%2");
  });
});

describe("join", () => {
  it("should combine path segments", () => {
    expect(join("https://host", "path", "segment")).toBe("https://host/path/segment");
    expect(join("https://host", "/path/", "/segment/")).toBe("https://host/path/segment");
    expect(join("https://host", "/path/", "/segment")).toBe("https://host/path/segment");
    expect(join("https://host", "path", "segment")).toBe("https://host/path/segment");
    expect(join("https://host/", "/path/", "/segment")).toBe("https://host/path/segment");
    expect(join("https://host/", "/path", "/")).toBe("https://host/path/");
    expect(join("/path/", "/segment/")).toBe("/path/segment");
    expect(join("/path/", "/segment")).toBe("/path/segment");
    expect(join("/path/", "/segment", "/")).toBe("/path/segment/");
    expect(join("/path/", "segment/", "file.txt")).toBe("/path/segment/file.txt");
    expect(join("path", "segment", "file.txt")).toBe("path/segment/file.txt");
  });
});

describe("getPathName function", () => {
  it("extracts file name from resource address", () => {
    expect(getPathName("folder/subfolder/file.txt")).toBe("file.txt");
    expect(getPathName("file.txt")).toBe("file.txt");
    expect(getPathName("folder/")).toBe("folder");
    expect(getPathName("folder/subfolder/")).toBe("subfolder");
  });

  it("decodes the last path segment", () => {
    expect(getPathName("folder/file%20name.txt")).toBe("file name.txt");
    expect(getPathName("path/sub%2Ffolder")).toBe("sub/folder");
  });
});

describe("getFileExtension function", () => {
  it("returns the file extension in uppercase", () => {
    expect(getFileExtension("file.txt")).toBe("TXT");
    expect(getFileExtension("image.PNG")).toBe("PNG");
    expect(getFileExtension("document.pdf")).toBe("PDF");
  });

  it("returns 'Unknown' when there's no extension", () => {
    expect(getFileExtension("filename")).toBe("Unknown");
    expect(getFileExtension("folder/")).toBe("Unknown");
  });

  it("does not throw on names containing literal percent signs", () => {
    expect(getFileExtension("https://host/container/object%25%25.jpg")).toBe("JPG");
    expect(getFileExtension("https://host/container/100%25.txt")).toBe("TXT");
  });
});

describe("getParents", () => {
  it("returns parents if present", () => {
    expect(getParents("")).toEqual([]);
    expect(getParents("https://example.com/")).toEqual([]);
    expect(getParents("https://example.com/foo")).toEqual(["https://example.com"]);
    expect(getParents("https://example.com/foo/")).toEqual(["https://example.com"]);
    expect(getParents("https://example.com/foo/bar")).toEqual(["https://example.com", "https://example.com/foo"]);
    expect(getParents("https://example.com/foo/bar/")).toEqual(["https://example.com", "https://example.com/foo"]);

    expect(getParents("omniverse://example.com/")).toEqual([]);
    expect(getParents("omniverse://example.com/foo")).toEqual(["omniverse://example.com"]);
    expect(getParents("omniverse://example.com/foo/")).toEqual(["omniverse://example.com"]);
    expect(getParents("omniverse://example.com/foo/bar")).toEqual([
      "omniverse://example.com",
      "omniverse://example.com/foo",
    ]);

    expect(getParents("/Test/")).toEqual(["/"]);
    expect(getParents("/Test/ABC")).toEqual(["/", "/Test"]);
    expect(getParents("/Test/ABC/DEF")).toEqual(["/", "/Test", "/Test/ABC"]);

    expect(getParents("Test")).toEqual([]);
    expect(getParents("Test/ABC")).toEqual(["Test"]);
    expect(getParents("Test/ABC/DEF")).toEqual(["Test", "Test/ABC"]);
  });
});
