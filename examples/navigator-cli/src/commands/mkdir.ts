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

import { noopLogger, printSuccess, type Logger } from "@/output";
import { type ApiInstance } from "@/services/api";
import { capabilitiesService } from "@/services/storage/capabilitiesService";
import { fileFolderService } from "@/services/storage/fileFolderService";
import { resolveStoragePath } from "@/util/resolve";
import { ensureFolderPath } from "@/util/url";

export async function runMkdir(
  api: ApiInstance,
  path: string,
  log: Logger = noopLogger(),
  cwd?: string,
): Promise<void> {
  const caps = capabilitiesService(api);
  const normalized = ensureFolderPath(
    await resolveStoragePath(caps, path, cwd),
  );
  log("mkdir", normalized);
  const service = fileFolderService(api);
  await service.createFolder(normalized);
  printSuccess(`Created ${normalized}`, { path: normalized });
}
