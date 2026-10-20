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

import { HTTPError } from "ky";
import { z } from "zod";
import { ApiInstance } from "@/services/api";
import { Path } from "@/state/path";
import { logger } from "@/util/logger";
import { fromSnakeCaseSchema } from "@/util/schemas";

export interface ArchiveItem {
  type: "file" | "folder";
  address: string;
}

export const PrepareResponseSchema = fromSnakeCaseSchema(
  z.object({
    downloadAction: z.string(),
    expiresAt: z.string(),
  })
);

export type PrepareResponse = z.infer<typeof PrepareResponseSchema>;

/**
 * REST client for the archive service. The service validates the selection and issues a
 * short-lived download cookie on prepare; the actual ZIP is streamed by a native form POST
 * to the returned download action (see triggerFormDownload).
 */
export function archiveService(api: ApiInstance) {
  return {
    async prepare(items: ArchiveItem[], archiveName: string, signal?: AbortSignal): Promise<PrepareResponse> {
      try {
        // credentials are required so the browser stores the Set-Cookie used by the download POST.
        const response = await api.post("archive/prepare", {
          json: { items, archive_name: archiveName },
          credentials: "include",
          signal,
        });
        return PrepareResponseSchema.parse(await response.json());
      } catch (error) {
        if (error instanceof HTTPError) {
          throw new Error(await problemDetail(error));
        }
        throw error;
      }
    },
  };
}

async function problemDetail(error: HTTPError): Promise<string> {
  try {
    const problem = await error.response.json<{ detail?: string; title?: string }>();
    return problem.detail ?? problem.title ?? `Failed to prepare archive -- HTTP${error.response.status}`;
  } catch {
    return `Failed to prepare archive -- HTTP${error.response.status}`;
  }
}

export function toArchiveItems(paths: Path[]): ArchiveItem[] {
  return paths.map((path) => ({
    type: path.isFolderLike ? "folder" : "file",
    address: path.resourceAddress,
  }));
}

/**
 * The archive download relies on a SameSite=Strict cookie, which the browser only sends when the
 * service shares a site with the app. Guards against a cross-site deployment where the native
 * download would silently fail, warning once so the caller can fall back to another download path.
 */
export function isArchiveServiceUsable(serviceBaseUrl: string, location: Location | URL = window.location): boolean {
  let serviceUrl: URL;
  try {
    serviceUrl = new URL(serviceBaseUrl);
  } catch {
    return false;
  }

  const appOrigin = `${location.protocol}//${location.host}`;
  if (serviceUrl.protocol === location.protocol && isSameSite(serviceUrl.hostname, location.hostname)) {
    return true;
  }

  logger.warn(
    `Archive service ${serviceBaseUrl} is on a different origin than the app ${appOrigin}; ` +
      `SameSite=Strict download cookies cannot be sent, so archive downloads are disabled.`
  );
  return false;
}

function isSameSite(serviceHost: string, appHost: string): boolean {
  if (serviceHost === appHost) {
    return true;
  }
  // Dev servers commonly run the app and service on different ports of localhost/an IP, which is
  // still same-site; skip the registrable-domain heuristic that does not apply to those hosts.
  if (isLocalOrIpHost(appHost) && isLocalOrIpHost(serviceHost)) {
    return true;
  }
  return registrableDomain(serviceHost) === registrableDomain(appHost);
}

function isLocalOrIpHost(host: string): boolean {
  return host === "localhost" || /^\d{1,3}(\.\d{1,3}){3}$/.test(host) || host.includes(":") || host.startsWith("[");
}

// Approximates the registrable domain (eTLD+1) as the trailing two labels of the host. Multi-part
// public suffixes (e.g. example.co.uk) are a known limitation; swap in a PSL library if it matters.
function registrableDomain(host: string): string {
  return host.split(".").slice(-2).join(".");
}

export interface TriggerFormDownloadParams {
  action: string;
  items: ArchiveItem[];
  archiveName: string;
}

/**
 * Starts a native browser download by submitting a hidden form to the archive download action in
 * the exact bracket notation the service expects. The form targets a hidden iframe so a pre-stream
 * error response never navigates the SPA away.
 */
export function triggerFormDownload({ action, items, archiveName }: TriggerFormDownloadParams): void {
  const iframeName = `archive-download-${Date.now()}`;
  const iframe = document.createElement("iframe");
  iframe.name = iframeName;
  iframe.style.display = "none";
  document.body.appendChild(iframe);

  const form = document.createElement("form");
  form.method = "POST";
  form.action = action;
  form.target = iframeName;
  form.enctype = "application/x-www-form-urlencoded";
  form.style.display = "none";

  items.forEach((item, index) => {
    appendHiddenField(form, `items[${index}][type]`, item.type);
    appendHiddenField(form, `items[${index}][address]`, item.address);
  });
  appendHiddenField(form, "archive_name", archiveName);

  document.body.appendChild(form);
  form.submit();

  // The browser detaches the download once response headers arrive, so the nodes can be removed
  // shortly after without interrupting the stream.
  window.setTimeout(() => {
    form.remove();
    iframe.remove();
  }, cleanupDelayMs);
}

function appendHiddenField(form: HTMLFormElement, name: string, value: string): void {
  const input = document.createElement("input");
  input.type = "hidden";
  input.name = name;
  input.value = value;
  form.appendChild(input);
}

const cleanupDelayMs = 60_000;
