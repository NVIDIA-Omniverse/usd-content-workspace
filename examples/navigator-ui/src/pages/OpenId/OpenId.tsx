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
import { Navigate, NavLink, useSearchParams } from "react-router-dom";
import { Button, Stack, Text, Title } from "@mantine/core";
import { LoaderError } from "@/components/LoaderError";
import { PageLoader } from "@/components/PageLoader";
import { useAddConnection } from "@/hooks/useAddConnection";
import { useServiceApi } from "@/hooks/useServiceApi";
import { ApiOptions } from "@/services/api";
import { decodeState, exchangeCode, getAuthConfig } from "@/services/auth";

export function OpenId() {
  const addConnection = useAddConnection();
  const { getApi } = useServiceApi();
  const [searchParams] = useSearchParams();

  const code = searchParams.get("code");
  const state = searchParams.get("state");
  const error = searchParams.get("error");
  const errorDescription = searchParams.get("error_description");

  const {
    isLoading,
    isError,
    error: internalError,
  } = useQuery({
    queryKey: ["openid", code, state],
    queryFn: async () => {
      if (!code) {
        throw new Error("`code` must be specified.", { cause: "auth callback" });
      }
      if (!state) {
        throw new Error("`state` must be specified.", { cause: "auth callback" });
      }

      const decodedState = decodeState(state);

      const config = await getAuthConfig(decodedState.urls.discovery);
      if (!config) {
        throw new Error("Failed to finish authentication -- authentication configuration is not available.");
      }

      const auth = await exchangeCode(code, config);
      const options: ApiOptions = {
        discovery: decodedState.urls.discovery,
        auth: {
          get: () => auth,
          set: () => {
            // no need for a set function here since we just completed authentication
          },
        },
      };
      try {
        const storage = await getApi(decodedState.urls.discovery, "filefolder", options);
        addConnection(decodedState.urls.discovery, storage ? [storage.baseUrl] : [], auth);
      } catch (error) {
        if (error instanceof Error) {
          throw new Error(error.message, { cause: "getStackInfo" });
        }
        throw error;
      }
      return null;
    },
    retry: false,
    retryOnMount: false,
    refetchOnMount: false,
    refetchOnReconnect: false,
    refetchInterval: false,
    refetchIntervalInBackground: false,
  });

  if (error) {
    return (
      <LoaderError title="Failed to authenticate">
        <Stack>
          <Title>{error}</Title>
          <Text>{errorDescription?.toString()}</Text>
          <NavLink to="/">
            <Button>Back to main page</Button>
          </NavLink>
        </Stack>
      </LoaderError>
    );
  }

  if (!code || !state) {
    return <Navigate to="/" />;
  }

  if (isLoading) {
    return <PageLoader label="Finishing authentication" />;
  }

  if (isError) {
    return (
      <LoaderError title="Failed to authenticate">
        <Stack>
          <Text>{internalError.toString()}</Text>
          <NavLink to="/">
            <Button>Back to main page</Button>
          </NavLink>
        </Stack>
      </LoaderError>
    );
  }

  return <Navigate to="/" />;
}
