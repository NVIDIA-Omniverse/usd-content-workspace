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
import { fetchServiceActions } from "../state/metadata";
import { useConfig } from "./useConfig";

export interface UseServiceActionsAutocompleteParams {
  service?: string;
}

export function useServiceActionsAutocomplete({
  service = "",
}: UseServiceActionsAutocompleteParams) {
  const config = useConfig();
  const auth = useAuth();

  const { data } = useQuery({
    queryKey: ["service-actions", service],
    queryFn: async () => {
      if (service === "" || service === "NULL") {
        return [];
      }
      return fetchServiceActions({ service, config, user: auth.user! });
    },
    enabled: !!service,
  });
  return data ?? [];
}
