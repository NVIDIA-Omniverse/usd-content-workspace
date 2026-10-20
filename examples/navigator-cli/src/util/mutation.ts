// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

import { CliError, type PreviewOperation } from "@/output";

export type ConflictPolicy = "fail" | "skip" | "overwrite";

export interface MutationOptions {
  dryRun?: boolean;
  onConflict?: ConflictPolicy;
}

export function parseConflictPolicy(value: string): ConflictPolicy {
  if (value === "fail" || value === "skip" || value === "overwrite")
    return value;
  throw new CliError(
    `Invalid conflict policy '${value}'. Expected fail, skip, or overwrite.`,
    "USAGE_ERROR",
  );
}

/** Keep command-wide progress when a later folder or file operation fails. */
export async function withMutationProgress<T>(
  action: (completed: PreviewOperation[]) => Promise<T>,
): Promise<T> {
  const completed: PreviewOperation[] = [];
  try {
    return await action(completed);
  } catch (error) {
    if (!completed.length) throw error;
    const message = error instanceof Error ? error.message : String(error);
    throw new CliError(message, "PARTIAL_FAILURE", {
      ...(error instanceof CliError
        ? error.details
        : { failures: [{ message }] }),
      completedOperations: completed,
    });
  }
}
