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

import { useQuery } from "@tanstack/react-query";
import {
  fetchGroup,
  formatGroupName,
  type GroupInfo,
} from "../api/userInfo.js";
import type { PermissionAuth } from "../types/PermissionAuth";
import type { PermissionConfig } from "../types/PermissionConfig";

export interface GroupNameResult {
  displayName: string | undefined;
  group: GroupInfo | undefined;
  isLoading: boolean;
}

export function useGroupName(
  id: string | undefined,
  auth: PermissionAuth,
  config: PermissionConfig,
): GroupNameResult {
  const hasUserService = !!config.userInfoUrl;

  const { data, isLoading } = useQuery({
    queryKey: ["group-name", id],
    queryFn: async () => fetchGroup({ groupId: id!, auth, config }),
    enabled: hasUserService && !!id,
    retry: false,
    staleTime: 5 * 60 * 1000,
  });

  return {
    displayName: data ? formatGroupName(data) : undefined,
    group: data,
    isLoading: hasUserService && !!id && isLoading,
  };
}
