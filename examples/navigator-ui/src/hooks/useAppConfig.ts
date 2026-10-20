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

import { useSuspenseQuery } from "@tanstack/react-query";
import { AppConfig, fetchAppConfig } from "@/services/appConfigService";

export const APP_CONFIG_QUERY_KEY = ["appConfig"] as const;

/**
 * Loads the SPA runtime config exactly once per session via Suspense.
 * The result is cached forever; the file is treated as immutable for the lifetime of the page.
 */
export function useAppConfig(): AppConfig {
  const { data } = useSuspenseQuery({
    queryKey: APP_CONFIG_QUERY_KEY,
    queryFn: fetchAppConfig,
    staleTime: Infinity,
    gcTime: Infinity,
    retry: false,
  });
  return data;
}
