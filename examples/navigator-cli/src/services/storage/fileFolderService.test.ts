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
import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";
import { create } from "@/services/api";
import { fileFolderService } from "./fileFolderService";

const folderAddress = "omniverse://nucleus/Library/";
const encoded = encodeURIComponent(folderAddress);

const server = setupServer(
  http.get(
    `https://storage.example.com/v1alpha/filefolder/list/${encoded}`,
    () =>
      HttpResponse.json({
        subfolder_addresses: ["omniverse://nucleus/Library/Sub/"],
        sub_resource_addresses: ["omniverse://nucleus/Library/scene.usd"],
        next_continuation_handle: null,
      }),
  ),
  http.put(
    `https://storage.example.com/v1alpha/filefolder/${encoded}`,
    () => new HttpResponse(null, { status: 204 }),
  ),
  http.delete(
    `https://storage.example.com/v1alpha/filefolder/list/${encoded}`,
    () => new HttpResponse(null, { status: 204 }),
  ),
);

beforeAll(() => server.listen());
afterAll(() => server.close());
afterEach(() => server.resetHandlers());

describe("fileFolderService", () => {
  const api = create("https://storage.example.com");
  const service = fileFolderService(api);

  it("list returns folder and file addresses", async () => {
    const result = await service.list(folderAddress, null);
    expect(result.subfolderAddresses).toContain(
      "omniverse://nucleus/Library/Sub/",
    );
    expect(result.subResourceAddresses).toContain(
      "omniverse://nucleus/Library/scene.usd",
    );
    expect(result.nextContinuationHandle).toBeNull();
  });

  it("createFolder sends PUT", async () => {
    await expect(service.createFolder(folderAddress)).resolves.toBeUndefined();
  });

  it("deleteFolder sends DELETE", async () => {
    await expect(service.deleteFolder(folderAddress)).resolves.toBeUndefined();
  });

  it("listAll collects results across multiple pages", async () => {
    let page = 0;
    server.use(
      http.get(
        `https://storage.example.com/v1alpha/filefolder/list/${encoded}`,
        ({ request }) => {
          const handle = new URL(request.url).searchParams.get(
            "continuation_handle",
          );
          if (!handle) {
            page = 1;
            return HttpResponse.json({
              subfolder_addresses: ["omniverse://nucleus/Library/Sub1/"],
              sub_resource_addresses: ["omniverse://nucleus/Library/a.usd"],
              next_continuation_handle: "page2",
            });
          }
          if (handle === "page2") {
            page = 2;
            return HttpResponse.json({
              subfolder_addresses: ["omniverse://nucleus/Library/Sub2/"],
              sub_resource_addresses: ["omniverse://nucleus/Library/b.usd"],
              next_continuation_handle: null,
            });
          }
        },
      ),
    );

    const result = await service.listAll(folderAddress);
    expect(page).toBe(2);
    expect(result.subfolderAddresses).toContain(
      "omniverse://nucleus/Library/Sub1/",
    );
    expect(result.subfolderAddresses).toContain(
      "omniverse://nucleus/Library/Sub2/",
    );
    expect(result.subResourceAddresses).toContain(
      "omniverse://nucleus/Library/a.usd",
    );
    expect(result.subResourceAddresses).toContain(
      "omniverse://nucleus/Library/b.usd",
    );
    expect(result.nextContinuationHandle).toBeNull();
  });
});
