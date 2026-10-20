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
import { useQuery } from "@tanstack/react-query";
import { useAuth } from "react-oidc-context";
import { fetchUser, type UserInfo } from "../state/users";
import { useConfig } from "./useConfig";

export interface PrincipalNameResult {
  displayName: string | undefined;
  user: UserInfo | undefined;
  isLoading: boolean;
}

/**
 * Resolves a principal identifier (user ID) to a human-readable display name
 * using the user info service. Falls back to the raw sub when the service is
 * unavailable or the user cannot be found.
 */
export function usePrincipalName(sub: string | undefined): PrincipalNameResult {
  const config = useConfig();
  const auth = useAuth();

  const hasUserService = !!config.endpoints.userService;

  const { data, isLoading } = useQuery({
    queryKey: ["principal-name", sub],
    queryFn: async () => {
      return fetchUser({ userId: sub!, config, user: auth.user! });
    },
    enabled: hasUserService && !!sub && !!auth.user,
    retry: false,
    staleTime: 5 * 60 * 1000,
  });

  return {
    displayName: data ? formatUserName(data) : undefined,
    user: data,
    isLoading: hasUserService && !!sub && isLoading,
  };
}

export function formatUserName(user: UserInfo): string {
  if (user.display_name) {
    return user.display_name;
  }
  if (user.email) {
    return user.email;
  }
  return user.id;
}
