/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
import type { Config } from "../../context/ConfigContext.ts";

export const mockConfig: Config = {
  auth: {
    clientId: "test-client-id",
    redirectUri: "http://localhost:3180/callback",
    authority: "https://auth.example.com",
    scope: "openid profile email",
  },
  endpoints: {
    backend: "http://localhost:8080",
  },
};

export const mockConfigWithUserService: Config = {
  ...mockConfig,
  endpoints: {
    ...mockConfig.endpoints,
    userService: "http://localhost:9090",
  },
};