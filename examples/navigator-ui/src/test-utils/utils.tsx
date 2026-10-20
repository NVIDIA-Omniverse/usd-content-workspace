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

import { QueryClient } from "@tanstack/react-query";
import { APP_CONFIG_QUERY_KEY } from "@/hooks/useAppConfig";
import { AppConfig } from "@/services/appConfigService";
import { Authentication } from "@/services/auth";

const testAppConfig: AppConfig = {
  defaultDiscovery: "",
  references: [],
  requestTimeout: 30000,
  batchSize: 25,
};

export function createTestQueryClient() {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: {
        retry: false,
      },
      mutations: {
        retry: false,
      },
    },
  });
  queryClient.setQueryData(APP_CONFIG_QUERY_KEY, testAppConfig);
  return queryClient;
}

export function createTestAuth(): Authentication {
  return {
    accessToken: "test-access-token",
    refreshToken: undefined,
    expiresAt: new Date(Date.now() + 24 * 60 * 60 * 1000),
    idToken: "test-id-token",
  };
}
