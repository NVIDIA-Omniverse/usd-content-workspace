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

import { z } from "zod";

/**
 * A reference link configured by a system administrator at deployment time.
 * Surfaced to end users in a dropdown menu next to the application menu.
 */
const ReferenceSchema = z.object({
  /** Text shown for the link in the references menu. */
  label: z.string(),
  /**
   * Destination opened in a new tab when the link is activated.
   * Restricted to absolute http(s) URLs or root-relative paths so unsafe schemes
   * (e.g. `javascript:`, `data:`) cannot be injected as an anchor href.
   */
  url: z.string().refine(isSafeReferenceUrl, { message: "must be an http(s) URL or a root-relative path" }),
  /**
   * Tabler webfont icon name (without the `ti-` prefix), e.g. `book` or `external-link`.
   * Rendered as a `ti ti-<icon>` element. When omitted or unknown, no glyph is shown.
   */
  icon: z.string().optional(),
});

export type AppReference = z.infer<typeof ReferenceSchema>;

/**
 * Runtime configuration for the SPA, delivered as a static `config.json`.
 * In production, the file is rendered by the Helm chart (see `helm/storage-navigator/templates/app-config.yaml`)
 * and mounted into the nginx static root. In development, `public/config.json` provides the same shape with empty defaults.
 */
const AppConfigSchema = z.object({
  /**
   * Discovery URL or hostname that Navigator should auto-connect to on load.
   * When non-empty, the app enters default-server mode: the manual "Add Connection"
   * UX is hidden and all storage services advertised by this discovery service are
   * surfaced as top-level nodes in the path tree.
   */
  defaultDiscovery: z.string().default(""),
  /**
   * Administrator-configured reference links shown to end users.
   * Defaults to an empty list when omitted; a present-but-malformed list fails validation
   * so deployment mistakes surface instead of being silently dropped.
   */
  references: z.array(ReferenceSchema).default([]),
  /**
   * Timeout in milliseconds for HTTP API requests made by the application.
   * Applies to all requests except file uploads, downloads, and transfers,
   * which always use an unlimited timeout. Defaults to 30000 (30 seconds).
   */
  requestTimeout: z.number().positive().max(300000).default(30000),
  /**
   * Maximum number of items processed in parallel per batch operation (e.g. delete, copy/paste).
   * Lower values reduce server load; higher values increase throughput. Defaults to 25.
   */
  batchSize: z.number().int().positive().max(500).default(25),
});

export type AppConfig = z.infer<typeof AppConfigSchema>;

export const APP_CONFIG_URL = "/config.json";

const EMPTY_CONFIG: AppConfig = { defaultDiscovery: "", references: [], requestTimeout: 30000, batchSize: 25 };

/**
 * Raised when a present `config.json` cannot be parsed or fails schema validation.
 * Carries human-readable `issues` (one per offending field) for display to operators.
 */
export class AppConfigError extends Error {
  readonly issues: string[];

  constructor(message: string, issues: string[] = []) {
    super(message);
    this.name = "AppConfigError";
    this.issues = issues;
  }
}

/**
 * Fetches and parses the SPA's runtime configuration.
 *
 * A missing file (HTTP 404) or network failure yields an empty config so the app still
 * boots in dev / unconfigured environments. Any other non-2xx status, invalid JSON, or
 * schema validation failure throws {@link AppConfigError} so a misconfiguration or
 * serving error is surfaced to operators rather than silently ignored.
 */
export async function fetchAppConfig(): Promise<AppConfig> {
  let response: Response;
  try {
    response = await fetch(APP_CONFIG_URL, { cache: "no-store" });
  } catch {
    return EMPTY_CONFIG;
  }

  if (response.status === 404) {
    return EMPTY_CONFIG;
  }

  if (!response.ok) {
    throw new AppConfigError(`Configuration file (config.json) could not be loaded (HTTP ${response.status}).`);
  }

  let raw: unknown;
  try {
    raw = await response.json();
  } catch {
    throw new AppConfigError("Configuration file (config.json) is not valid JSON.");
  }

  const result = AppConfigSchema.safeParse(raw);
  if (!result.success) {
    throw new AppConfigError("Configuration file (config.json) failed validation.", formatIssues(result.error));
  }

  return result.data;
}

function formatIssues(error: z.ZodError): string[] {
  return error.issues.map((issue) => `${issue.path.join(".") || "(root)"}: ${issue.message}`);
}

function isSafeReferenceUrl(value: string): boolean {
  if (value.startsWith("/") && !value.startsWith("//")) {
    return true;
  }

  try {
    return ["http:", "https:"].includes(new URL(value).protocol);
  } catch {
    return false;
  }
}
