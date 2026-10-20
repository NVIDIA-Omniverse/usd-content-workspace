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

import { http, HttpResponse } from "msw";
import { setupServer } from "msw/node";
import {
  afterAll,
  afterEach,
  beforeAll,
  beforeEach,
  describe,
  expect,
  it,
  vi,
} from "vitest";
import { create } from "@/services/api";
import {
  runThumbnail,
  fetchThumbnailBytes,
  detectImageProtocol,
  renderInlineImage,
} from "./thumbnail";

const fileAddress = "omniverse://nucleus/Library/scene.usd";
const folderAddress = "omniverse://nucleus/Library/";
const thumbAddress = "omniverse://nucleus/.thumbnails/scene.usd.jpg";
const thumbIdentity = `${thumbAddress}?version-id=abc`;
const thumbIdentityEncoded = encodeURIComponent(thumbIdentity);

const topLevelAddress = "https://mybucket.blob.core.windows.net/mybucket/";
const shortFileAddress = `${topLevelAddress}Library/scene.usd`;

const fakeImageBytes = Buffer.from([0xff, 0xd8, 0xff, 0xe0]); // JPEG magic bytes

// Shape of a GET /v1alpha/thumbnails entry in the thumbnail service OpenAPI.
const thumbnailEntry = {
  target_type: "FILE",
  association_target: { resource_address: fileAddress },
  thumbnail: {
    resource_address: thumbAddress,
    resource_identity: thumbIdentity,
  },
  specification: {
    type: "IMAGE_TO_IMAGE",
    width: 256,
    height: 256,
    encoding_format: "JPEG",
    quality: null,
  },
  create_time: "2026-04-01T12:00:00Z",
};

const thumbnailQueryParams = new Set([
  "association_target_resource_address",
  "specification",
  "width",
  "height",
  "encoding_format",
  "transform_type",
  "crop_strategy",
]);

// Parsed form of `thumbnailEntry`, as `fetchThumbnailBytes` receives it.
const parsedEntry = {
  thumbnail: {
    resourceAddress: thumbAddress,
    resourceIdentity: thumbIdentity,
  },
  specification: null,
  createTime: null,
};

const thumbnailRequests: URL[] = [];

const server = setupServer(
  http.get(
    "https://storage.example.com/v1alpha/capabilities/top-level-addresses",
    () =>
      HttpResponse.json({
        items: [
          {
            top_level_address:
              "https://mybucket.blob.core.windows.net/mybucket",
          },
        ],
      }),
  ),
  http.get("https://thumb.example.com/v1alpha/thumbnails", ({ request }) => {
    const url = new URL(request.url);
    thumbnailRequests.push(url);
    // The real service rejects query parameters it does not know.
    if (
      [...url.searchParams.keys()].some((k) => !thumbnailQueryParams.has(k))
    ) {
      return HttpResponse.json(
        { source: "thumbnail", error_message: "unknown query parameter" },
        { status: 400 },
      );
    }
    const target = url.searchParams.get("association_target_resource_address");
    if (target === fileAddress || target === shortFileAddress) {
      return HttpResponse.json({ thumbnails: [thumbnailEntry] });
    }
    return HttpResponse.json({ thumbnails: [] });
  }),
  http.get(
    `https://storage.example.com/v1alpha/fileobject/by-identity/${thumbIdentityEncoded}`,
    () =>
      new HttpResponse(fakeImageBytes, {
        status: 200,
        headers: { "Content-Type": "image/jpeg" },
      }),
  ),
);

beforeAll(() => server.listen());
afterAll(() => server.close());
afterEach(() => server.resetHandlers());

describe("runThumbnail", () => {
  const storageApi = create("https://storage.example.com");
  const thumbApi = create("https://thumb.example.com");

  beforeEach(() => {
    thumbnailRequests.length = 0;
    vi.stubEnv("TERM_PROGRAM", "");
    vi.stubEnv("ITERM_SESSION_ID", "");
    vi.stubEnv("KITTY_WINDOW_ID", "");
    vi.stubEnv("TERM", "");
  });
  afterEach(() => {
    vi.unstubAllEnvs();
  });

  it("prints thumbnail info including a data URL", async () => {
    const lines: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((msg) => {
      lines.push(String(msg));
      return true;
    });
    await runThumbnail(storageApi, thumbApi, fileAddress, { json: false });
    const out = lines.join("");
    expect(out).toContain(thumbIdentity);
    expect(out).toContain(thumbAddress);
    expect(out).toContain("2026-04-01T12:00:00Z");
    expect(out).toContain("data:image/jpeg;base64,");
  });

  it("requests the 256x256 thumbnail for the resolved address", async () => {
    vi.spyOn(process.stdout, "write").mockImplementation(() => true);
    await runThumbnail(storageApi, thumbApi, fileAddress, { json: false });
    expect(thumbnailRequests).toHaveLength(1);
    expect(Object.fromEntries(thumbnailRequests[0].searchParams)).toEqual({
      association_target_resource_address: fileAddress,
      width: "256",
      height: "256",
    });
  });

  it("queries a folder by its address", async () => {
    vi.spyOn(process.stdout, "write").mockImplementation(() => true);
    await runThumbnail(storageApi, thumbApi, folderAddress, { json: false });
    expect(
      thumbnailRequests[0].searchParams.get(
        "association_target_resource_address",
      ),
    ).toBe(folderAddress);
  });

  it("outputs JSON without data URLs when json flag set", async () => {
    const lines: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((msg) => {
      lines.push(String(msg));
      return true;
    });
    await runThumbnail(storageApi, thumbApi, fileAddress, { json: true });
    const parsed = JSON.parse(lines.join("")) as {
      thumbnail: { resourceAddress: string; resourceIdentity: string };
      createTime: string;
      specification: unknown;
    }[];
    expect(Array.isArray(parsed)).toBe(true);
    expect(parsed[0].thumbnail).toEqual({
      resourceAddress: thumbAddress,
      resourceIdentity: thumbIdentity,
    });
    expect(parsed[0].createTime).toBe("2026-04-01T12:00:00Z");
    expect(parsed[0].specification).toEqual(thumbnailEntry.specification);
    expect(lines.join("")).not.toContain("data:");
  });

  it("prints no-thumbnails message when result is empty", async () => {
    const lines: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((msg) => {
      lines.push(String(msg));
      return true;
    });
    await runThumbnail(storageApi, thumbApi, folderAddress, { json: false });
    expect(lines.join("")).toContain("No thumbnails found");
  });

  it("resolves a short name to its full address", async () => {
    const lines: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((msg) => {
      lines.push(String(msg));
      return true;
    });
    await runThumbnail(storageApi, thumbApi, "mybucket/Library/scene.usd", {
      json: false,
    });
    expect(lines.join("")).toContain("data:image/jpeg;base64,");
  });

  it("resolves a bare name relative to cwd", async () => {
    const cwd = "https://mybucket.blob.core.windows.net/mybucket/Library/";
    const lines: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((msg) => {
      lines.push(String(msg));
      return true;
    });
    await runThumbnail(storageApi, thumbApi, "scene.usd", { json: false }, cwd);
    expect(lines.join("")).toContain("data:image/jpeg;base64,");
  });

  it("omits URL line gracefully when image fetch fails", async () => {
    server.use(
      http.get(
        `https://storage.example.com/v1alpha/fileobject/by-identity/${thumbIdentityEncoded}`,
        () => new HttpResponse(null, { status: 500 }),
      ),
    );
    const lines: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((msg) => {
      lines.push(String(msg));
      return true;
    });
    await runThumbnail(storageApi, thumbApi, fileAddress, { json: false });
    const out = lines.join("");
    expect(out).toContain(thumbIdentity);
    expect(out).not.toContain("data:");
  });

  it("renders inline image via iTerm2 when TERM_PROGRAM=iTerm.app", async () => {
    vi.stubEnv("TERM_PROGRAM", "iTerm.app");
    const lines: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((msg) => {
      lines.push(String(msg));
      return true;
    });
    await runThumbnail(storageApi, thumbApi, fileAddress, { json: false });
    const out = lines.join("");
    expect(out).toContain("\x1b]1337;File=inline=1");
    expect(out).not.toContain("data:image/");
  });

  it("falls back to data URL in Kitty when image is JPEG", async () => {
    vi.stubEnv("KITTY_WINDOW_ID", "1");
    const lines: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((msg) => {
      lines.push(String(msg));
      return true;
    });
    await runThumbnail(storageApi, thumbApi, fileAddress, { json: false });
    const out = lines.join("");
    expect(out).not.toContain("\x1b_G");
    expect(out).toContain("data:image/jpeg;base64,");
  });

  it("renders inline image via Kitty when image is PNG", async () => {
    vi.stubEnv("KITTY_WINDOW_ID", "1");
    server.use(
      http.get(
        `https://storage.example.com/v1alpha/fileobject/by-identity/${thumbIdentityEncoded}`,
        () =>
          new HttpResponse(
            new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
            {
              status: 200,
              headers: { "Content-Type": "image/png" },
            },
          ),
      ),
    );
    const lines: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((msg) => {
      lines.push(String(msg));
      return true;
    });
    await runThumbnail(storageApi, thumbApi, fileAddress, { json: false });
    const out = lines.join("");
    expect(out).toContain("\x1b_Ga=T,f=100");
    expect(out).not.toContain("data:image/");
  });
});

describe("detectImageProtocol", () => {
  afterEach(() => {
    vi.unstubAllEnvs();
  });

  it("returns iterm2 for iTerm.app", () => {
    vi.stubEnv("TERM_PROGRAM", "iTerm.app");
    expect(detectImageProtocol()).toBe("iterm2");
  });

  it("returns iterm2 for WezTerm", () => {
    vi.stubEnv("TERM_PROGRAM", "WezTerm");
    expect(detectImageProtocol()).toBe("iterm2");
  });

  it("returns kitty for ghostty", () => {
    vi.stubEnv("TERM_PROGRAM", "ghostty");
    expect(detectImageProtocol()).toBe("kitty");
  });

  it("returns iterm2 when ITERM_SESSION_ID is set", () => {
    vi.stubEnv("TERM_PROGRAM", "");
    vi.stubEnv("ITERM_SESSION_ID", "w0t0p0:abc");
    expect(detectImageProtocol()).toBe("iterm2");
  });

  it("returns kitty when KITTY_WINDOW_ID is set", () => {
    vi.stubEnv("TERM_PROGRAM", "");
    vi.stubEnv("KITTY_WINDOW_ID", "1");
    expect(detectImageProtocol()).toBe("kitty");
  });

  it("returns kitty for xterm-kitty TERM", () => {
    vi.stubEnv("TERM_PROGRAM", "");
    vi.stubEnv("KITTY_WINDOW_ID", "");
    vi.stubEnv("TERM", "xterm-kitty");
    expect(detectImageProtocol()).toBe("kitty");
  });

  it("returns null when no terminal support", () => {
    vi.stubEnv("TERM_PROGRAM", "");
    vi.stubEnv("ITERM_SESSION_ID", "");
    vi.stubEnv("KITTY_WINDOW_ID", "");
    vi.stubEnv("TERM", "xterm-256color");
    expect(detectImageProtocol()).toBeNull();
  });

  it("prefers iterm2 over kitty when both env vars are set", () => {
    vi.stubEnv("TERM_PROGRAM", "WezTerm");
    vi.stubEnv("KITTY_WINDOW_ID", "1");
    expect(detectImageProtocol()).toBe("iterm2");
  });
});

describe("renderInlineImage", () => {
  it("emits OSC 1337 escape sequence for iterm2", () => {
    const lines: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((msg) => {
      lines.push(String(msg));
      return true;
    });
    const buf = Buffer.from([0xff, 0xd8, 0xff, 0xe0]);
    const result = renderInlineImage(buf, "image/jpeg", "iterm2");
    const out = lines.join("");
    expect(result).toBe(true);
    expect(out).toContain("\x1b]1337;File=inline=1;preserveAspectRatio=1:");
    expect(out).toContain(buf.toString("base64"));
    expect(out).toContain("\x07");
  });

  it("emits Kitty APC sequence for PNG", () => {
    const lines: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((msg) => {
      lines.push(String(msg));
      return true;
    });
    const buf = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
    const result = renderInlineImage(buf, "image/png", "kitty");
    const out = lines.join("");
    expect(result).toBe(true);
    expect(out).toContain("\x1b_Ga=T,f=100,q=2");
    expect(out).toContain("\x1b\\");
  });

  it("returns false for kitty with non-PNG", () => {
    const buf = Buffer.from([0xff, 0xd8, 0xff, 0xe0]);
    expect(renderInlineImage(buf, "image/jpeg", "kitty")).toBe(false);
  });

  it("sends multiple APC chunks for large PNG", () => {
    const lines: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((msg) => {
      lines.push(String(msg));
      return true;
    });
    // 4096 base64 chars = ~3072 raw bytes, so 4000 raw bytes forces 2 chunks
    const buf = Buffer.alloc(4000, 0x89); // 4000 bytes, all PNG-ish
    const result = renderInlineImage(buf, "image/png", "kitty");
    const out = lines.join("");
    expect(result).toBe(true);
    // First chunk: m=1 (more data coming)
    expect(out).toContain("m=1;");
    // Final chunk: m=0 (last chunk)
    expect(out).toContain("m=0;");
    // Both APC start and ST terminator present
    expect(out).toContain("\x1b_G");
    expect(out).toContain("\x1b\\");
  });

  it("returns false for empty buffer", () => {
    const buf = Buffer.alloc(0);
    expect(renderInlineImage(buf, "image/png", "iterm2")).toBe(false);
    expect(renderInlineImage(buf, "image/png", "kitty")).toBe(false);
  });
});

describe("fetchThumbnailBytes", () => {
  const storageApi = create("https://storage.example.com");

  it("returns buffer and mimeType on 200", async () => {
    const result = await fetchThumbnailBytes(storageApi, parsedEntry);
    expect(result).not.toBeNull();
    expect(result!.mimeType).toBe("image/jpeg");
    expect(result!.buffer).toBeInstanceOf(Buffer);
    expect(result!.buffer.length).toBeGreaterThan(0);
  });

  it("downloads by resource identity, not by address", async () => {
    const requested: string[] = [];
    server.use(
      http.get(
        "https://storage.example.com/v1alpha/fileobject/:kind/:resource",
        ({ params }) => {
          requested.push(`${String(params.kind)}/${String(params.resource)}`);
          return new HttpResponse(fakeImageBytes, {
            status: 200,
            headers: { "Content-Type": "image/jpeg" },
          });
        },
      ),
    );
    await fetchThumbnailBytes(storageApi, parsedEntry);
    // msw hands back the decoded path parameter.
    expect(requested).toEqual([`by-identity/${thumbIdentity}`]);
  });

  it("returns null on non-200/300 response", async () => {
    server.use(
      http.get(
        `https://storage.example.com/v1alpha/fileobject/by-identity/${thumbIdentityEncoded}`,
        () => new HttpResponse(null, { status: 500 }),
      ),
    );
    const result = await fetchThumbnailBytes(storageApi, parsedEntry);
    expect(result).toBeNull();
  });

  it("falls back to the thumbnail address extension when response content-type is application/octet-stream", async () => {
    server.use(
      http.get(
        `https://storage.example.com/v1alpha/fileobject/by-identity/${thumbIdentityEncoded}`,
        () =>
          new HttpResponse(fakeImageBytes, {
            status: 200,
            headers: { "Content-Type": "application/octet-stream" },
          }),
      ),
    );
    const result = await fetchThumbnailBytes(storageApi, parsedEntry);
    expect(result).not.toBeNull();
    expect(result!.mimeType).toBe("image/jpeg");

    const pngEntry = {
      ...parsedEntry,
      thumbnail: {
        ...parsedEntry.thumbnail,
        resourceAddress: "omniverse://nucleus/.thumbnails/scene.usd.png",
      },
    };
    const pngResult = await fetchThumbnailBytes(storageApi, pngEntry);
    expect(pngResult!.mimeType).toBe("image/png");
  });

  it("follows a 300 redirect and uses the thumbnail address extension when it has no content-type", async () => {
    let redirectHeader: string | null = null;
    server.use(
      http.get(
        `https://storage.example.com/v1alpha/fileobject/by-identity/${thumbIdentityEncoded}`,
        () =>
          HttpResponse.json(
            {
              redirect_target_url: "https://blob.example.com/scene.usd.png",
              additional_headers: { "x-ms-blob-type": "BlockBlob" },
            },
            { status: 300 },
          ),
      ),
      http.get("https://blob.example.com/scene.usd.png", ({ request }) => {
        redirectHeader = request.headers.get("x-ms-blob-type");
        return new HttpResponse(fakeImageBytes, { status: 200 });
      }),
    );
    const pngEntry = {
      ...parsedEntry,
      thumbnail: {
        ...parsedEntry.thumbnail,
        resourceAddress: "omniverse://nucleus/.thumbnails/scene.usd.png",
      },
    };
    const result = await fetchThumbnailBytes(storageApi, pngEntry);
    expect(redirectHeader).toBe("BlockBlob");
    expect(result).not.toBeNull();
    expect(result!.mimeType).toBe("image/png");
  });
});
