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
import { createContext } from "react";
import { z } from "zod";

/**
 * Defines the validation schema for the application configuration stored in /config.json file.
 */
export const Config = z.object({
  auth: z.object({
    // OpenID Client ID registered in the identity provider for this application.
    clientId: z.string().min(1, "Required"),

    // The URI of this application where the user will be redirected back after authentication.
    redirectUri: z.string().url(),

    // The URI used for redirecting the user to the identity provider.
    authority: z.string().min(1, "Required"),

    // Scopes required from the identity provider.
    // Define what fields (claims) become available in the ID token.
    scope: z.string().default("openid profile email"),
  }),

  endpoints: z.object({
    // The URI of the portal backend.
    backend: z.string().min(1, "Required"),

    // The URI of the user info service. Optional — user and group pages are disabled when not set.
    userService: z.string().optional(),
  }),
});

export type Config = z.infer<typeof Config>;

export const ConfigContext = createContext<Config>({} as unknown as Config);
