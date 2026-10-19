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
import { Loader } from "@mantine/core";
import { useQuery } from "@tanstack/react-query";
import { useAuth } from "react-oidc-context";
import { useParams } from "react-router-dom";
import LoaderError from "../components/LoaderError";
import { useConfig } from "../hooks/useConfig";
import { fetchPolicy } from "../state/policies";
import { PolicyForm } from "./PolicyForm";

export function PolicyEdit() {
  const config = useConfig();
  const auth = useAuth();
  const params = useParams<{ policyId: string }>();

  const { isLoading, isError, error, isSuccess, data } = useQuery({
    queryKey: ["policy-details", params.policyId],
    queryFn: async () => {
      return fetchPolicy({
        policyId: params.policyId!,
        config,
        user: auth.user!,
      });
    },
    enabled: !!params.policyId,
  });

  if (isLoading) {
    return <Loader />;
  }

  if (isError) {
    return <LoaderError>{error.message}</LoaderError>;
  }

  if (isSuccess) {
    return <PolicyForm initial={data} />;
  }

  return null;
}
