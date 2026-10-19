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

import { Progress } from "ky";
import { http, HttpResponse } from "msw";
import { setupServer } from "msw/node";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ApiInstance, create as createApi } from "@/services/api";
import { DownloadRedirectResponse, fileObjectService, RedirectResponse, UploadResponse } from "./fileObjectService";

describe("fileObjectService", () => {
  const putRequestSpy = vi.fn();
  const prepareRequestSpy = vi.fn();
  const completeRequestSpy = vi.fn();

  const server = setupServer();

  beforeEach(() => {
    server.resetHandlers();
    server.listen();
  });

  afterEach(() => {
    server.close();
  });

  it("should perform a multipart upload successfully", async () => {
    const resourceAddress = "my/test/address";

    const firstPartRedirect: RedirectResponse = {
      redirectTargetUrl: "https://upload-part-1",
      additionalHeaders: [],
      method: "PUT",
      completionHeaderNames: ["etag"],
    } as const;

    const multipartInfo = {
      uploadId: "123e4567-e89b-12d3-a456-426614174000",
      firstPartWriteRedirect: firstPartRedirect,
      minimumSizePerPart: 1,
      maximumSizePerPart: 1024,
      maximumPartsNumber: 10,
    } as NonNullable<UploadResponse["multipart"]>;

    const secondPartRedirect: RedirectResponse = {
      redirectTargetUrl: "https://upload-part-2",
      additionalHeaders: [],
      method: "PUT",
      completionHeaderNames: ["etag"],
    };

    server.use(
      http.put("http://test-service/v1alpha/fileobject/by-address/*", () => {
        putRequestSpy();
        return HttpResponse.json({ multipart: multipartInfo }, { status: 300 });
      }),

      http.post("http://test-service/v1alpha/fileobject/by-address/*/multipart/prepare", async ({ request }) => {
        const payload = await request.json();
        prepareRequestSpy(payload);
        return HttpResponse.json({ partWriteRedirects: [secondPartRedirect] }, { status: 200 });
      }),

      http.post("http://test-service/v1alpha/fileobject/by-address/*/multipart/complete", async ({ request }) => {
        const payload = await request.json();
        completeRequestSpy(payload);
        return new Response(null, { status: 200 });
      }),

      http.put("https://upload-part-1", () => new Response(null, { status: 200, headers: { etag: "part1" } })),
      http.put("https://upload-part-2", () => new Response(null, { status: 200, headers: { etag: "part2" } }))
    );

    const fileContent = new Uint8Array(2048).fill(1);

    const file = new File([fileContent], "test.bin", { type: "application/octet-stream" });

    const abortController = new AbortController();

    const service = fileObjectService(createApi("http://test-service/v1alpha"));

    await service.upload(resourceAddress, file as unknown as File, () => void 0, abortController.signal);

    expect(putRequestSpy).toHaveBeenCalledTimes(1);
    expect(prepareRequestSpy).toHaveBeenCalledTimes(1);
    expect(completeRequestSpy).toHaveBeenCalledTimes(1);

    const preparePayload = prepareRequestSpy.mock.calls[0][0];
    expect(preparePayload).toEqual({
      upload_id: multipartInfo.uploadId,
      part_number: 1,
      part_count: 1,
    });

    const completePayload = completeRequestSpy.mock.calls[0][0] as {
      upload_id: string;
      parts: { part_number: number }[];
    };
    expect(completePayload.upload_id).toBe(multipartInfo.uploadId);
    expect(completePayload.parts).toEqual(
      expect.arrayContaining([expect.objectContaining({ part_number: 0 }), expect.objectContaining({ part_number: 1 })])
    );
  });

  it("should call complete after a redirect upload", async () => {
    const resourceAddress = "my/redirect/address";

    const redirect: RedirectResponse = {
      redirectTargetUrl: "https://upload-redirect-target",
      additionalHeaders: [],
      method: "PUT",
      completionHeaderNames: ["etag"],
    } as const;

    const redirectUploadSpy = vi.fn();
    const redirectCompleteSpy = vi.fn();

    server.use(
      http.put("http://test-service/v1alpha/fileobject/by-address/*", () => {
        return HttpResponse.json({ redirect }, { status: 300 });
      }),

      http.put(redirect.redirectTargetUrl, () => {
        redirectUploadSpy();
        return new Response(null, { status: 200, headers: { etag: "abc123" } });
      }),

      http.post("http://test-service/v1alpha/fileobject/by-address/*/redirect/complete", async ({ request }) => {
        const payload = await request.json();
        redirectCompleteSpy(payload);
        return new Response(null, { status: 200 });
      })
    );

    const file = new File([new Uint8Array(16).fill(1)], "test.bin", { type: "application/octet-stream" });

    const abortController = new AbortController();

    const service = fileObjectService(createApi("http://test-service/v1alpha"));

    await service.upload(resourceAddress, file as unknown as File, () => void 0, abortController.signal);

    expect(redirectUploadSpy).toHaveBeenCalledTimes(1);
    expect(redirectCompleteSpy).toHaveBeenCalledTimes(1);
    expect(redirectCompleteSpy).toHaveBeenCalledWith({
      additional_headers: [{ name: "etag", value: "abc123" }],
    });
  });

  it("should report the final probe progress when the service accepts a direct upload", async () => {
    const put = vi.fn(
      (_path: string, options: { onUploadProgress?: (progress: Progress) => void }): Promise<Response> => {
        options.onUploadProgress?.({ percent: 0.5, transferredBytes: 8, totalBytes: 16 });
        options.onUploadProgress?.({ percent: 1, transferredBytes: 16, totalBytes: 16 });
        return Promise.resolve({ status: 201 } as Response);
      }
    );
    const onProgress = vi.fn();
    const service = fileObjectService({ put } as unknown as ApiInstance);

    await service.upload(
      "my/direct/address",
      new File([new Uint8Array(16)], "test.bin"),
      onProgress,
      new AbortController().signal
    );

    expect(onProgress.mock.calls.map((call) => (call[0] as Progress).transferredBytes)).toEqual([16]);
  });

  it("should not report probe progress when the upload is redirected", async () => {
    const redirect: RedirectResponse = {
      redirectTargetUrl: "https://upload-redirect-target",
      additionalHeaders: [],
      method: "PUT",
      completionHeaderNames: [],
    };
    const put = vi.fn(
      (_path: string, options: { onUploadProgress?: (progress: Progress) => void }): Promise<Response> => {
        options.onUploadProgress?.({ percent: 0.5, transferredBytes: 8, totalBytes: 16 });
        return Promise.resolve({
          status: 300,
          json: () =>
            Promise.resolve({
              redirect: {
                redirect_target_url: redirect.redirectTargetUrl,
                additional_headers: [],
                method: redirect.method,
                completion_header_names: [],
              },
            }),
        } as Response);
      }
    );
    const post = vi.fn().mockResolvedValue({ status: 200 });
    server.use(http.put(redirect.redirectTargetUrl, () => new Response(null, { status: 200 })));
    const onProgress = vi.fn();

    const service = fileObjectService({ put, post } as unknown as ApiInstance);
    await service.upload(
      "my/redirect/address",
      new File([new Uint8Array(16)], "test.bin"),
      onProgress,
      new AbortController().signal
    );

    const reportedBytes = onProgress.mock.calls.map((call) => (call[0] as Progress).transferredBytes);
    expect(reportedBytes).not.toContain(8);
    expect(reportedBytes.length).toBeGreaterThan(0);
  });

  it("should call complete when the redirect upload returns a 2xx other than 200", async () => {
    const resourceAddress = "my/redirect/address";

    const redirect: RedirectResponse = {
      redirectTargetUrl: "https://upload-redirect-target-201",
      additionalHeaders: [],
      method: "PUT",
      completionHeaderNames: ["etag"],
    } as const;

    const redirectCompleteSpy = vi.fn();

    server.use(
      http.put("http://test-service/v1alpha/fileobject/by-address/*", () => {
        return HttpResponse.json({ redirect }, { status: 300 });
      }),

      http.put(redirect.redirectTargetUrl, () => {
        return new Response(null, { status: 201, headers: { etag: "abc123" } });
      }),

      http.post("http://test-service/v1alpha/fileobject/by-address/*/redirect/complete", async ({ request }) => {
        const payload = await request.json();
        redirectCompleteSpy(payload);
        return new Response(null, { status: 200 });
      })
    );

    const file = new File([new Uint8Array(16).fill(1)], "test.bin", { type: "application/octet-stream" });

    const abortController = new AbortController();

    const service = fileObjectService(createApi("http://test-service/v1alpha"));

    await service.upload(resourceAddress, file as unknown as File, () => void 0, abortController.signal);

    expect(redirectCompleteSpy).toHaveBeenCalledTimes(1);
  });

  it("should fetch metadata and resource identity via stat", async () => {
    const resourceAddress = "folder/file.txt";

    const metadataPayload = {
      data_object_size: 4321,
      last_modified_timestamp: "2024-05-15T12:34:56Z",
    } as const;

    const resourceIdentity = "folder/file.txt?content-length=6120252&version-id=2025-08-06T18%3A09%3A59.4703318Z";

    const headRequestSpy = vi.fn();

    server.use(
      http.head("http://test-service/v1alpha/fileobject/by-address/*", () => {
        headRequestSpy();
        return new Response(null, {
          status: 204,
          headers: {
            "X-Nvidia-Omniverse-Storage-Metadata": JSON.stringify(metadataPayload),
            "X-Nvidia-Omniverse-Storage-Resource-Identity": resourceIdentity,
          },
        });
      })
    );

    const service = fileObjectService(createApi("http://test-service/v1alpha"));

    const statResult = await service.stat(resourceAddress);

    expect(headRequestSpy).toHaveBeenCalledTimes(1);
    expect(statResult).toEqual({
      metadata: {
        dataObjectSize: metadataPayload.data_object_size,
        lastModifiedTimestamp: metadataPayload.last_modified_timestamp,
      },
      resourceIdentity,
    });
  });

  it.each([
    {
      resourceType: "address" as const,
      resource: "redirect-test/file.txt",
      expectedPathPrefix: "by-address",
      unexpectedPathPrefix: "by-identity",
    },
    {
      resourceType: "identity" as const,
      resource: "redirect-test/file.txt?version-id=2025-08-06T18%3A09%3A59.4703318Z",
      expectedPathPrefix: "by-identity",
      unexpectedPathPrefix: "by-address",
    },
  ])(
    "should download by $resourceType via redirect and include additional headers",
    async ({ resourceType, resource, expectedPathPrefix, unexpectedPathPrefix }) => {
      const redirectInfo: DownloadRedirectResponse = {
        redirectTargetUrl: "https://redirect-target.com/file.txt",
        additionalHeaders: {
          "X-Custom": "custom-value",
        },
      } as const;

      const expectedRequestSpy = vi.fn();
      const unexpectedRequestSpy = vi.fn();
      const redirectRequestSpy = vi.fn();

      const binaryContent = new Uint8Array([0xde, 0xad, 0xbe, 0xef]);

      server.use(
        http.get(`http://test-service/v1alpha/fileobject/${expectedPathPrefix}/*`, () => {
          expectedRequestSpy();
          return HttpResponse.json(redirectInfo, { status: 300 });
        }),

        http.get(`http://test-service/v1alpha/fileobject/${unexpectedPathPrefix}/*`, () => {
          unexpectedRequestSpy();
          return new Response(null, { status: 500 });
        }),

        http.get(redirectInfo.redirectTargetUrl, ({ request }) => {
          redirectRequestSpy(request.headers.get("X-Custom"));
          return new Response(binaryContent, {
            status: 200,
            headers: { "Content-Type": "application/octet-stream" },
          });
        })
      );

      const service = fileObjectService(createApi("http://test-service/v1alpha"));
      const abortController = new AbortController();
      const response = await service.download(resource, resourceType, { signal: abortController.signal });

      expect(expectedRequestSpy).toHaveBeenCalledTimes(1);
      expect(unexpectedRequestSpy).not.toHaveBeenCalled();
      expect(redirectRequestSpy).toHaveBeenCalledWith("custom-value");

      expect(response.status).toBe(200);

      const blob = await response.blob();
      expect(blob.size).toBe(binaryContent.length);

      const arrayBuffer = await blob.arrayBuffer();
      expect(new Uint8Array(arrayBuffer)).toEqual(binaryContent);
    }
  );

  it.each([
    {
      operation: "copy",
      route: "http://test-service/v1alpha/fileobject/by-identity/*/copy",
      status: 200,
      failIfExists: true,
    },
    {
      operation: "copy",
      route: "http://test-service/v1alpha/fileobject/by-identity/*/copy",
      status: 200,
      failIfExists: false,
    },
    {
      operation: "move",
      route: "http://test-service/v1alpha/fileobject/by-address/*/move",
      status: 201,
      failIfExists: true,
    },
    {
      operation: "move",
      route: "http://test-service/v1alpha/fileobject/by-address/*/move",
      status: 201,
      failIfExists: false,
    },
  ] as const)("should pass fail_if_exists=$failIfExists when calling $operation", async (testCase) => {
    const destinationAddress = "folder/new-name.txt";
    const requestSpy = vi.fn();

    server.use(
      http.post(testCase.route, async ({ request }) => {
        requestSpy(await request.json());
        return HttpResponse.json({ resource_identity: `${destinationAddress}-identity` }, { status: testCase.status });
      })
    );

    const service = fileObjectService(createApi("http://test-service/v1alpha"));
    await service[testCase.operation]("source", destinationAddress, testCase.failIfExists);

    expect(requestSpy).toHaveBeenCalledWith({
      destination_resource_address: destinationAddress,
      fail_if_exists: testCase.failIfExists,
    });
  });

  it("should move a file object to a new address", async () => {
    const sourceAddress = "folder/old-name.txt";
    const destinationAddress = "folder/new-name.txt";
    const resourceIdentity = "folder/new-name.txt?version-id=2025-11-17T10%3A30%3A00.123Z";

    const moveRequestSpy = vi.fn();

    server.use(
      http.post("http://test-service/v1alpha/fileobject/by-address/*/move", async ({ request }) => {
        const payload = await request.json();
        moveRequestSpy(payload);
        return HttpResponse.json({ resource_identity: resourceIdentity }, { status: 201 });
      })
    );

    const service = fileObjectService(createApi("http://test-service/v1alpha"));
    const result = await service.move(sourceAddress, destinationAddress);

    expect(moveRequestSpy).toHaveBeenCalledTimes(1);
    expect(moveRequestSpy).toHaveBeenCalledWith({
      destination_resource_address: destinationAddress,
    });
    expect(result.resourceIdentity).toBe(resourceIdentity);
  });

  describe("upload retries", () => {
    beforeEach(() => {
      vi.useFakeTimers();
    });

    afterEach(() => {
      vi.useRealTimers();
    });

    it.each([408, 429, 503])(
      "retries the direct upload probe after a transient HTTP %i and eventually succeeds",
      async (status) => {
        const resourceAddress = "my/flaky/address";
        let attempts = 0;

        server.use(
          http.put("http://test-service/v1alpha/fileobject/by-address/*", () => {
            attempts += 1;
            if (attempts < 3) {
              return new Response(null, { status });
            }
            return new Response(null, { status: 201 });
          })
        );

        const service = fileObjectService(createApi("http://test-service/v1alpha"));
        const abortController = new AbortController();

        const uploadPromise = service.upload(
          resourceAddress,
          new File([new Uint8Array(16)], "test.bin"),
          () => void 0,
          abortController.signal
        );
        await vi.runAllTimersAsync();
        await uploadPromise;

        expect(attempts).toBe(3);
      }
    );

    it("retries the direct upload probe after a network failure", async () => {
      const resourceAddress = "my/network-flaky/address";
      let attempts = 0;

      server.use(
        http.put("http://test-service/v1alpha/fileobject/by-address/*", () => {
          attempts += 1;
          return attempts === 1 ? HttpResponse.error() : new Response(null, { status: 201 });
        })
      );

      const service = fileObjectService(createApi("http://test-service/v1alpha"));
      const uploadPromise = service.upload(
        resourceAddress,
        new File([new Uint8Array(16)], "test.bin"),
        () => void 0,
        new AbortController().signal
      );
      await vi.runAllTimersAsync();
      await uploadPromise;

      expect(attempts).toBe(2);
    });

    it("gives up after exhausting retries on a persistent 5xx", async () => {
      const resourceAddress = "my/persistently-flaky/address";
      let attempts = 0;

      server.use(
        http.put("http://test-service/v1alpha/fileobject/by-address/*", () => {
          attempts += 1;
          return new Response(null, { status: 503 });
        })
      );

      const service = fileObjectService(createApi("http://test-service/v1alpha"));
      const abortController = new AbortController();

      const uploadPromise = service.upload(
        resourceAddress,
        new File([new Uint8Array(16)], "test.bin"),
        () => void 0,
        abortController.signal
      );
      const assertion = expect(uploadPromise).rejects.toThrow(/HTTP503/);
      await vi.runAllTimersAsync();
      await assertion;

      expect(attempts).toBe(3); // initial attempt + ky's 2 default retries
    });

    it("does not retry the direct upload probe on a non-retryable error", async () => {
      const resourceAddress = "my/broken/address";
      let attempts = 0;

      server.use(
        http.put("http://test-service/v1alpha/fileobject/by-address/*", () => {
          attempts += 1;
          return new Response(null, { status: 400 });
        })
      );

      const service = fileObjectService(createApi("http://test-service/v1alpha"));
      const abortController = new AbortController();

      await expect(
        service.upload(
          resourceAddress,
          new File([new Uint8Array(16)], "test.bin"),
          () => void 0,
          abortController.signal
        )
      ).rejects.toThrow(/HTTP400/);

      expect(attempts).toBe(1);
    });

    it("rejects a direct upload when it has already been aborted", async () => {
      const resourceAddress = "my/aborted/address";

      server.use(
        http.put("http://test-service/v1alpha/fileobject/by-address/*", () => {
          return new Response(null, { status: 201 });
        })
      );

      const service = fileObjectService(createApi("http://test-service/v1alpha"));
      const abortController = new AbortController();
      abortController.abort();

      await expect(
        service.upload(
          resourceAddress,
          new File([new Uint8Array(16)], "test.bin"),
          () => void 0,
          abortController.signal
        )
      ).rejects.toMatchObject({ name: "AbortError" });
    });

    it("retries a redirect upload after a transient HTTP 503 and eventually succeeds", async () => {
      const resourceAddress = "my/flaky-redirect/address";

      const redirect: RedirectResponse = {
        redirectTargetUrl: "https://upload-redirect-target-flaky",
        additionalHeaders: [],
        method: "PUT",
        completionHeaderNames: ["etag"],
      };

      let redirectAttempts = 0;

      server.use(
        http.put("http://test-service/v1alpha/fileobject/by-address/*", () =>
          HttpResponse.json({ redirect }, { status: 300 })
        ),
        http.put(redirect.redirectTargetUrl, () => {
          redirectAttempts += 1;
          if (redirectAttempts < 2) {
            return new Response(null, { status: 503 });
          }
          return new Response(null, { status: 200, headers: { etag: "abc123" } });
        }),
        http.post("http://test-service/v1alpha/fileobject/by-address/*/redirect/complete", () => {
          return new Response(null, { status: 200 });
        })
      );

      const service = fileObjectService(createApi("http://test-service/v1alpha"));
      const abortController = new AbortController();

      const uploadPromise = service.upload(
        resourceAddress,
        new File([new Uint8Array(16).fill(1)], "test.bin", { type: "application/octet-stream" }),
        () => void 0,
        abortController.signal
      );
      await vi.runAllTimersAsync();
      await uploadPromise;

      expect(redirectAttempts).toBe(2);
    });

    it("retries a redirect upload after a network failure and eventually succeeds", async () => {
      const resourceAddress = "my/network-flaky-redirect/address";
      const redirect: RedirectResponse = {
        redirectTargetUrl: "https://upload-redirect-target-network-flaky",
        additionalHeaders: [],
        method: "PUT",
        completionHeaderNames: ["etag"],
      };
      let redirectAttempts = 0;

      server.use(
        http.put("http://test-service/v1alpha/fileobject/by-address/*", () =>
          HttpResponse.json({ redirect }, { status: 300 })
        ),
        http.put(redirect.redirectTargetUrl, () => {
          redirectAttempts += 1;
          if (redirectAttempts === 1) {
            return HttpResponse.error();
          }
          return new Response(null, { status: 200, headers: { etag: "abc123" } });
        }),
        http.post("http://test-service/v1alpha/fileobject/by-address/*/redirect/complete", () => {
          return new Response(null, { status: 200 });
        })
      );

      const service = fileObjectService(createApi("http://test-service/v1alpha"));
      const uploadPromise = service.upload(
        resourceAddress,
        new File([new Uint8Array(16)], "test.bin"),
        () => void 0,
        new AbortController().signal
      );
      await vi.runAllTimersAsync();
      await uploadPromise;

      expect(redirectAttempts).toBe(2);
    });

    it("rejects a redirected upload with the abort reason", async () => {
      const resourceAddress = "my/aborted-redirect/address";
      const redirect: RedirectResponse = {
        redirectTargetUrl: "https://upload-redirect-target-aborted",
        additionalHeaders: [],
        method: "PUT",
        completionHeaderNames: [],
      };
      const abortController = new AbortController();
      const abortReason = new DOMException("Upload cancelled", "AbortError");

      server.use(
        http.put("http://test-service/v1alpha/fileobject/by-address/*", () =>
          HttpResponse.json({ redirect }, { status: 300 })
        ),
        http.put(redirect.redirectTargetUrl, () => {
          abortController.abort(abortReason);
          return new Response(null, { status: 200 });
        })
      );

      const service = fileObjectService(createApi("http://test-service/v1alpha"));
      await expect(
        service.upload(
          resourceAddress,
          new File([new Uint8Array(16)], "test.bin"),
          () => void 0,
          abortController.signal
        )
      ).rejects.toBe(abortReason);
    });

    it("retries an individual multipart chunk upload after a transient HTTP 429 and eventually succeeds", async () => {
      const resourceAddress = "my/flaky-multipart/address";

      const firstPartRedirect: RedirectResponse = {
        redirectTargetUrl: "https://upload-flaky-part-1",
        additionalHeaders: [],
        method: "PUT",
        completionHeaderNames: ["etag"],
      };

      const multipartInfo = {
        uploadId: "123e4567-e89b-12d3-a456-426614174001",
        firstPartWriteRedirect: firstPartRedirect,
        minimumSizePerPart: 1,
        maximumSizePerPart: 2048,
        maximumPartsNumber: 10,
      } as NonNullable<UploadResponse["multipart"]>;

      let partAttempts = 0;

      server.use(
        http.put("http://test-service/v1alpha/fileobject/by-address/*", () =>
          HttpResponse.json({ multipart: multipartInfo }, { status: 300 })
        ),
        http.put(firstPartRedirect.redirectTargetUrl, () => {
          partAttempts += 1;
          if (partAttempts < 2) {
            return new Response(null, { status: 429 });
          }
          return new Response(null, { status: 200, headers: { etag: "part1" } });
        }),
        http.post("http://test-service/v1alpha/fileobject/by-address/*/multipart/complete", () => {
          return new Response(null, { status: 200 });
        })
      );

      const fileContent = new Uint8Array(1024).fill(1);
      const file = new File([fileContent], "test.bin", { type: "application/octet-stream" });

      const service = fileObjectService(createApi("http://test-service/v1alpha"));
      const abortController = new AbortController();

      const uploadPromise = service.upload(resourceAddress, file, () => void 0, abortController.signal);
      await vi.runAllTimersAsync();
      await uploadPromise;

      expect(partAttempts).toBe(2);
    });

    it("retries an individual multipart chunk after a network failure", async () => {
      const resourceAddress = "my/network-flaky-multipart/address";
      const firstPartRedirect: RedirectResponse = {
        redirectTargetUrl: "https://upload-network-flaky-part-1",
        additionalHeaders: [],
        method: "PUT",
        completionHeaderNames: ["etag"],
      };
      const multipartInfo = {
        uploadId: "123e4567-e89b-12d3-a456-426614174002",
        firstPartWriteRedirect: firstPartRedirect,
        minimumSizePerPart: 1,
        maximumSizePerPart: 2048,
        maximumPartsNumber: 10,
      } as NonNullable<UploadResponse["multipart"]>;
      let partAttempts = 0;

      server.use(
        http.put("http://test-service/v1alpha/fileobject/by-address/*", () =>
          HttpResponse.json({ multipart: multipartInfo }, { status: 300 })
        ),
        http.put(firstPartRedirect.redirectTargetUrl, () => {
          partAttempts += 1;
          if (partAttempts === 1) {
            return HttpResponse.error();
          }
          return new Response(null, { status: 200, headers: { etag: "part1" } });
        }),
        http.post("http://test-service/v1alpha/fileobject/by-address/*/multipart/complete", () => {
          return new Response(null, { status: 200 });
        })
      );

      const service = fileObjectService(createApi("http://test-service/v1alpha"));
      const uploadPromise = service.upload(
        resourceAddress,
        new File([new Uint8Array(1024)], "test.bin"),
        () => void 0,
        new AbortController().signal
      );
      await vi.runAllTimersAsync();
      await uploadPromise;

      expect(partAttempts).toBe(2);
    });

    it("gives up after exhausting retries on a persistently failing redirect upload", async () => {
      const resourceAddress = "my/persistently-flaky-redirect/address";
      const redirect: RedirectResponse = {
        redirectTargetUrl: "https://upload-redirect-target-persistently-flaky",
        additionalHeaders: [],
        method: "PUT",
        completionHeaderNames: ["etag"],
      };
      let redirectAttempts = 0;

      server.use(
        http.put("http://test-service/v1alpha/fileobject/by-address/*", () =>
          HttpResponse.json({ redirect }, { status: 300 })
        ),
        http.put(redirect.redirectTargetUrl, () => {
          redirectAttempts += 1;
          return new Response(null, { status: 503 });
        })
      );

      const service = fileObjectService(createApi("http://test-service/v1alpha"));
      const abortController = new AbortController();

      const uploadPromise = service.upload(
        resourceAddress,
        new File([new Uint8Array(16)], "test.bin"),
        () => void 0,
        abortController.signal
      );
      const assertion = expect(uploadPromise).rejects.toThrow(/HTTP503/);
      await vi.runAllTimersAsync();
      await assertion;

      expect(redirectAttempts).toBe(3); // initial attempt + 2 retries
    });

    it("does not retry a redirect upload on a non-retryable error", async () => {
      const resourceAddress = "my/broken-redirect/address";
      const redirect: RedirectResponse = {
        redirectTargetUrl: "https://upload-redirect-target-broken",
        additionalHeaders: [],
        method: "PUT",
        completionHeaderNames: ["etag"],
      };
      let redirectAttempts = 0;

      server.use(
        http.put("http://test-service/v1alpha/fileobject/by-address/*", () =>
          HttpResponse.json({ redirect }, { status: 300 })
        ),
        http.put(redirect.redirectTargetUrl, () => {
          redirectAttempts += 1;
          return new Response(null, { status: 400 });
        })
      );

      const service = fileObjectService(createApi("http://test-service/v1alpha"));
      const abortController = new AbortController();

      await expect(
        service.upload(
          resourceAddress,
          new File([new Uint8Array(16)], "test.bin"),
          () => void 0,
          abortController.signal
        )
      ).rejects.toThrow(/HTTP400/);

      expect(redirectAttempts).toBe(1);
    });

    it("rejects a redirect upload when it has already been aborted", async () => {
      const resourceAddress = "my/already-aborted-redirect/address";
      const redirect: RedirectResponse = {
        redirectTargetUrl: "https://upload-redirect-target-already-aborted",
        additionalHeaders: [],
        method: "PUT",
        completionHeaderNames: ["etag"],
      };

      server.use(
        http.put("http://test-service/v1alpha/fileobject/by-address/*", () =>
          HttpResponse.json({ redirect }, { status: 300 })
        )
      );

      const service = fileObjectService(createApi("http://test-service/v1alpha"));
      const abortController = new AbortController();
      abortController.abort();

      await expect(
        service.upload(
          resourceAddress,
          new File([new Uint8Array(16)], "test.bin"),
          () => void 0,
          abortController.signal
        )
      ).rejects.toMatchObject({ name: "AbortError" });
    });

    it("gives up after exhausting retries on a persistently failing multipart chunk", async () => {
      const resourceAddress = "my/persistently-flaky-multipart/address";
      const firstPartRedirect: RedirectResponse = {
        redirectTargetUrl: "https://upload-persistently-flaky-part-1",
        additionalHeaders: [],
        method: "PUT",
        completionHeaderNames: ["etag"],
      };
      const multipartInfo = {
        uploadId: "123e4567-e89b-12d3-a456-426614174003",
        firstPartWriteRedirect: firstPartRedirect,
        minimumSizePerPart: 1,
        maximumSizePerPart: 2048,
        maximumPartsNumber: 10,
      } as NonNullable<UploadResponse["multipart"]>;
      let partAttempts = 0;

      server.use(
        http.put("http://test-service/v1alpha/fileobject/by-address/*", () =>
          HttpResponse.json({ multipart: multipartInfo }, { status: 300 })
        ),
        http.put(firstPartRedirect.redirectTargetUrl, () => {
          partAttempts += 1;
          return new Response(null, { status: 503 });
        })
      );

      const service = fileObjectService(createApi("http://test-service/v1alpha"));
      const abortController = new AbortController();

      const uploadPromise = service.upload(
        resourceAddress,
        new File([new Uint8Array(1024)], "test.bin"),
        () => void 0,
        abortController.signal
      );
      const assertion = expect(uploadPromise).rejects.toThrow(/HTTP503/);
      await vi.runAllTimersAsync();
      await assertion;

      expect(partAttempts).toBe(3); // initial attempt + 2 retries
    });

    it("does not retry a multipart chunk on a non-retryable error", async () => {
      const resourceAddress = "my/broken-multipart/address";
      const firstPartRedirect: RedirectResponse = {
        redirectTargetUrl: "https://upload-broken-part-1",
        additionalHeaders: [],
        method: "PUT",
        completionHeaderNames: ["etag"],
      };
      const multipartInfo = {
        uploadId: "123e4567-e89b-12d3-a456-426614174004",
        firstPartWriteRedirect: firstPartRedirect,
        minimumSizePerPart: 1,
        maximumSizePerPart: 2048,
        maximumPartsNumber: 10,
      } as NonNullable<UploadResponse["multipart"]>;
      let partAttempts = 0;

      server.use(
        http.put("http://test-service/v1alpha/fileobject/by-address/*", () =>
          HttpResponse.json({ multipart: multipartInfo }, { status: 300 })
        ),
        http.put(firstPartRedirect.redirectTargetUrl, () => {
          partAttempts += 1;
          return new Response(null, { status: 400 });
        })
      );

      const service = fileObjectService(createApi("http://test-service/v1alpha"));
      const abortController = new AbortController();

      await expect(
        service.upload(
          resourceAddress,
          new File([new Uint8Array(1024)], "test.bin"),
          () => void 0,
          abortController.signal
        )
      ).rejects.toThrow(/HTTP400/);

      expect(partAttempts).toBe(1);
    });
  });
});
