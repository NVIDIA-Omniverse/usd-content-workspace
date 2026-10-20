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
import { Alert, Loader } from "@mantine/core";
import { useQuery } from "@tanstack/react-query";
import { useAuth } from "react-oidc-context";
import { Navigate, useParams } from "react-router-dom";
import { Page } from "../components/Page";
import { useConfig } from "../hooks/useConfig";
import { fetchFullServiceInfo } from "../state/metadata";
import { ServiceForm } from "./ServiceForm";

export function ServiceDetails() {
  const { service = "" } = useParams<{ service: string }>();
  const config = useConfig();
  const auth = useAuth();

  const { isLoading, isError, error, isSuccess, data } = useQuery({
    queryKey: ["service", service],
    queryFn: async () => {
      return fetchFullServiceInfo({
        service: service!,
        config,
        user: auth.user!,
      });
    },
    enabled: !!service,
  });

  if (!service) {
    return <Navigate to={"/services"} />;
  }

  if (isLoading) {
    return (
      <Page title={service}>
        <Loader />
      </Page>
    );
  }

  if (isError) {
    return (
      <Page title={service}>
        <Alert color={"red"}>{error.message}</Alert>
      </Page>
    );
  }

  if (isSuccess) {
    return <ServiceForm title={service} initialValues={data} />;
  }

  return <Navigate to={"/services"} />;
}
