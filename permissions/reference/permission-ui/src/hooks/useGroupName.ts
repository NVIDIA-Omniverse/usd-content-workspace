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
import { fetchGroup, type GroupInfo } from "../state/users";
import { useConfig } from "./useConfig";

export interface GroupNameResult {
  displayName: string | undefined;
  group: GroupInfo | undefined;
  isLoading: boolean;
}

/**
 * Resolves a group identifier to a human-readable display name using the user
 * info service. Falls back to the raw id when the service is unavailable or the
 * group cannot be found.
 */
export function useGroupName(id: string | undefined): GroupNameResult {
  const config = useConfig();
  const auth = useAuth();

  const hasUserService = !!config.endpoints.userService;

  const { data, isLoading } = useQuery({
    queryKey: ["group-name", id],
    queryFn: async () => {
      return fetchGroup({ groupId: id!, config, user: auth.user! });
    },
    enabled: hasUserService && !!id && !!auth.user,
    retry: false,
    staleTime: 5 * 60 * 1000,
  });

  return {
    displayName: data ? formatGroupName(data) : undefined,
    group: data,
    isLoading: hasUserService && !!id && isLoading,
  };
}

export function formatGroupName(group: GroupInfo): string {
  if (group.display_name) {
    return group.display_name;
  }
  return group.id;
}
