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
import { vi } from "vitest";
import type { User } from "oidc-client-ts";

vi.mock("react-oidc-context", () => ({
  useAuth: () => ({
    user: mockUser,
    isAuthenticated: true,
    isLoading: false,
    error: null,
    activeNavigator: null,
    events: {
      addSilentRenewError: vi.fn(),
      removeSilentRenewError: vi.fn(),
    },
    signinSilent: vi.fn().mockResolvedValue(null),
    signinRedirect: vi.fn(),
  }),
  hasAuthParams: () => false,
}));

export const mockUser = {
  access_token: "mock-access-token",
  profile: {
    sub: "user-123",
    email: "test@example.com",
    name: "Test User",
  },
  expires_at: Date.now() / 1000 + 3600,
  token_type: "Bearer",
  scope: "openid profile email",
} as User;
