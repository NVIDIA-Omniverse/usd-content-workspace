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

import { encodeResourceUrl, getResourceAncestors } from "./resources.js";
import type { ResourceAddress } from "../types/ResourceAddress";

describe("encodeResourceUrl", () => {
  test("alphanumeric characters remain unchanged", () => {
    expect(encodeResourceUrl("http://localhost:3000/abc123")).toBe(
      "http://localhost:3000/abc123",
    );
    expect(encodeResourceUrl("http://localhost:3000/ABCdef")).toBe(
      "http://localhost:3000/ABCdef",
    );
  });

  test("unreserved special characters remain unchanged", () => {
    expect(encodeResourceUrl("http://localhost:3000/file.txt")).toBe(
      "http://localhost:3000/file.txt",
    );
    expect(encodeResourceUrl("http://localhost:3000/my-file")).toBe(
      "http://localhost:3000/my-file",
    );
    expect(encodeResourceUrl("http://localhost:3000/my_file")).toBe(
      "http://localhost:3000/my_file",
    );
    expect(encodeResourceUrl("http://localhost:3000/my~file")).toBe(
      "http://localhost:3000/my~file",
    );
    expect(encodeResourceUrl("http://localhost:3000/my.file")).toBe(
      "http://localhost:3000/my.file",
    );
  });

  test("multiple unreserved characters in sequence", () => {
    expect(encodeResourceUrl("http://localhost:3000/abc-123_def.txt")).toBe(
      "http://localhost:3000/abc-123_def.txt",
    );
  });

  test("special characters are percent-encoded", () => {
    expect(encodeResourceUrl("http://localhost:3000/my file.txt")).toBe(
      "http://localhost:3000/my%20file.txt",
    );
    expect(encodeResourceUrl("http://localhost:3000/file&1.txt")).toBe(
      "http://localhost:3000/file%261.txt",
    );
    expect(encodeResourceUrl("http://localhost:3000/file+1.txt")).toBe(
      "http://localhost:3000/file%2B1.txt",
    );
    expect(encodeResourceUrl("http://localhost:3000/file=1.txt")).toBe(
      "http://localhost:3000/file%3D1.txt",
    );
    expect(encodeResourceUrl("http://localhost:3000/file@1.txt")).toBe(
      "http://localhost:3000/file%401.txt",
    );
    expect(encodeResourceUrl("http://localhost:3000/file[1.txt")).toBe(
      "http://localhost:3000/file%5B1.txt",
    );
    expect(encodeResourceUrl("http://localhost:3000/file]1.txt")).toBe(
      "http://localhost:3000/file%5D1.txt",
    );
    expect(encodeResourceUrl("http://localhost:3000/file{1.txt")).toBe(
      "http://localhost:3000/file%7B1.txt",
    );
    expect(encodeResourceUrl("http://localhost:3000/file}1.txt")).toBe(
      "http://localhost:3000/file%7D1.txt",
    );
    expect(encodeResourceUrl("http://localhost:3000/file|1.txt")).toBe(
      "http://localhost:3000/file%7C1.txt",
    );
    expect(encodeResourceUrl("http://localhost:3000/file^1.txt")).toBe(
      "http://localhost:3000/file%5E1.txt",
    );
    expect(encodeResourceUrl("http://localhost:3000/file`1.txt")).toBe(
      "http://localhost:3000/file%601.txt",
    );
  });

  test("Unicode characters are percent-encoded", () => {
    expect(encodeResourceUrl("http://localhost:3000/файл.txt")).toBe(
      "http://localhost:3000/%D1%84%D0%B0%D0%B9%D0%BB.txt",
    );
    expect(encodeResourceUrl("http://localhost:3000/文件.txt")).toBe(
      "http://localhost:3000/%E6%96%87%E4%BB%B6.txt",
    );
  });

  test("mixed reserved and unreserved characters", () => {
    expect(encodeResourceUrl("http://localhost:3000/my file-name.txt")).toBe(
      "http://localhost:3000/my%20file-name.txt",
    );
    expect(
      encodeResourceUrl("http://localhost:3000/file with spaces & symbols.txt"),
    ).toBe("http://localhost:3000/file%20with%20spaces%20%26%20symbols.txt");
  });

  test("empty pathname", () => {
    expect(encodeResourceUrl("http://localhost:3000/")).toBe(
      "http://localhost:3000/",
    );
  });

  test("pathname with only unreserved characters", () => {
    expect(encodeResourceUrl("http://localhost:3000/abc123")).toBe(
      "http://localhost:3000/abc123",
    );
  });

  test("complex nested paths", () => {
    expect(
      encodeResourceUrl(
        "http://localhost:3000/folder with spaces/sub folder/file name.txt",
      ),
    ).toBe(
      "http://localhost:3000/folder%20with%20spaces/sub%20folder/file%20name.txt",
    );
  });

  test("URLs with query parameters and fragments", () => {
    expect(
      encodeResourceUrl("http://localhost:3000/file.txt?param=value#fragment"),
    ).toBe("http://localhost:3000/file.txt?param=value#fragment");
  });

  test("query parameters remain unchanged", () => {
    expect(
      encodeResourceUrl(
        "http://localhost:3000/file with spaces.txt?param=value with spaces&other=123",
      ),
    ).toBe(
      "http://localhost:3000/file%20with%20spaces.txt?param=value%20with%20spaces&other=123",
    );
    expect(
      encodeResourceUrl(
        "http://localhost:3000/path/file.txt?search=test&filter=active",
      ),
    ).toBe("http://localhost:3000/path/file.txt?search=test&filter=active");
  });

  test("fragment remains unchanged", () => {
    expect(
      encodeResourceUrl(
        "http://localhost:3000/file with spaces.txt#section with spaces",
      ),
    ).toBe(
      "http://localhost:3000/file%20with%20spaces.txt#section%20with%20spaces",
    );
    expect(
      encodeResourceUrl("http://localhost:3000/path/file.txt#section1"),
    ).toBe("http://localhost:3000/path/file.txt#section1");
  });

  test("query parameters and fragments with special characters remain unchanged", () => {
    expect(
      encodeResourceUrl(
        "http://localhost:3000/file.txt?param=value#&fragment=test",
      ),
    ).toBe("http://localhost:3000/file.txt?param=value#&fragment=test");
    expect(
      encodeResourceUrl(
        "http://localhost:3000/path/file.txt?search=test+value&filter=active#section=1",
      ),
    ).toBe(
      "http://localhost:3000/path/file.txt?search=test+value&filter=active#section=1",
    );
  });

  test("complex URL with all components", () => {
    expect(
      encodeResourceUrl(
        "https://api.example.com:8443/v1/users/profile with spaces?token=abc123&expires=2024#section=details",
      ),
    ).toBe(
      "https://api.example.com:8443/v1/users/profile%20with%20spaces?token=abc123&expires=2024#section=details",
    );
  });

  test("URL with encoded characters in path should not be double-encoded", () => {
    expect(
      encodeResourceUrl(
        "http://localhost:3000/Users/test%40email.com/Test%20File.txt",
      ),
    ).toBe("http://localhost:3000/Users/test%40email.com/Test%20File.txt");
  });

  test("URL with encoded characters in query/fragment should not be double-encoded", () => {
    expect(
      encodeResourceUrl(
        "http://localhost:3000/file.txt?param=value%20with%20spaces#fragment%20with%20spaces",
      ),
    ).toBe(
      "http://localhost:3000/file.txt?param=value%20with%20spaces#fragment%20with%20spaces",
    );
  });
});

describe("getResourceAncestors", () => {
  test("returns selection then parent folders up to root for an object", () => {
    const resource: ResourceAddress = {
      address: "https://exampleaccount.blob.core.windows.net/container/folder/",
      type: "object",
    };

    expect(getResourceAncestors(resource)).toEqual([
      resource,
      {
        address: "https://exampleaccount.blob.core.windows.net/container/",
        type: "folder",
      },
      {
        address: "https://exampleaccount.blob.core.windows.net/",
        type: "folder",
      },
    ]);
  });

  test("handles an object without a trailing slash", () => {
    const resource: ResourceAddress = {
      address: "https://example.com/container/folder",
      type: "object",
    };

    expect(getResourceAncestors(resource)).toEqual([
      resource,
      { address: "https://example.com/container/", type: "folder" },
      { address: "https://example.com/", type: "folder" },
    ]);
  });

  test("returns only the selection when it is already the root", () => {
    const resource: ResourceAddress = {
      address: "https://example.com/",
      type: "folder",
    };

    expect(getResourceAncestors(resource)).toEqual([resource]);
  });

  test("handles non-special schemes such as omniverse://", () => {
    const resource: ResourceAddress = {
      address: "omniverse://example-server/workspace/",
      type: "folder",
    };

    expect(getResourceAncestors(resource)).toEqual([
      resource,
      { address: "omniverse://example-server/", type: "folder" },
    ]);
  });

  test("walks multiple omniverse path segments up to the server root", () => {
    const resource: ResourceAddress = {
      address: "omniverse://example-server/workspace/file.usd",
      type: "object",
    };

    expect(getResourceAncestors(resource)).toEqual([
      resource,
      {
        address: "omniverse://example-server/workspace/",
        type: "folder",
      },
      { address: "omniverse://example-server/", type: "folder" },
    ]);
  });

  test("drops query parameters and fragments from ancestors", () => {
    const resource: ResourceAddress = {
      address: "https://example.com/a/b/?token=abc#frag",
      type: "folder",
    };

    expect(getResourceAncestors(resource)).toEqual([
      resource,
      { address: "https://example.com/a/", type: "folder" },
      { address: "https://example.com/", type: "folder" },
    ]);
  });
});
