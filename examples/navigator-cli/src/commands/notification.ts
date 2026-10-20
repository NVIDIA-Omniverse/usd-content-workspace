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

// src/commands/notification.ts
import chalk from "chalk";
import { type Logger, noopLogger } from "@/output";
import { type ApiInstance } from "@/services/api";
import { notificationService } from "@/services/notificationService";

function safeDisplay(value: unknown, fallback = ""): string {
  if (typeof value === "string") {
    return value;
  }
  if (typeof value === "number" || typeof value === "boolean") {
    return String(value);
  }
  return fallback;
}

export function formatNotificationEvent(event: unknown, json: boolean): string {
  if (json) {
    return JSON.stringify(event);
  }
  const e = event as Record<string, unknown>;
  const eventType = safeDisplay(e.event_type, "unknown");
  const who = safeDisplay(e.principal_identity);
  const when = safeDisplay(e.occurred_at);
  const msg = e.message as Record<string, unknown> | undefined;
  const location =
    typeof msg?.resource_address === "string"
      ? msg.resource_address
      : JSON.stringify(msg ?? {});
  return `${chalk.cyan(`[${eventType}]`)} ${location}  ${chalk.dim(`(${who} · ${when})`)}`;
}

export async function runNotification(
  api: ApiInstance,
  cwd: string | undefined,
  eventTypes: string[],
  options: { json: boolean },
  log: Logger = noopLogger(),
  signal?: AbortSignal,
): Promise<void> {
  if (!cwd) {
    throw new Error("no current folder set. Use 'sn cd <path>' to set one.");
  }

  log("notification", `subscribing to ${eventTypes.join(", ")} at ${cwd}`);

  return new Promise<void>((resolve, reject) => {
    const service = notificationService(api);

    const client = service.createEventSource({
      path: cwd,
      eventTypes,
      onConnected: () => {
        if (!options.json) {
          process.stderr.write(
            `Watching ${chalk.bold(cwd)} for: ${eventTypes.join(", ")}\n`,
          );
        }
      },
      onMessage: (event) => {
        process.stdout.write(
          `${formatNotificationEvent(event, options.json)}\n`,
        );
      },
      onError: (err) => {
        client.close();
        reject(err);
      },
    });

    const sigintHandler = () => {
      client.close();
      resolve();
    };
    process.once("SIGINT", sigintHandler);

    signal?.addEventListener("abort", () => {
      process.removeListener("SIGINT", sigintHandler);
      client.close();
      resolve();
    });
  });
}
