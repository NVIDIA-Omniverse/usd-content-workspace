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
import { List, Loader, Text } from "@mantine/core";
import { useQuery } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { ZodError } from "zod";
import LoaderError from "../components/LoaderError";
import { Config, ConfigContext } from "../context/ConfigContext";

export interface ConfigProviderProps {
  children?: ReactNode;
}

/**
 * Loads the application configuration and passes it as context.
 * While the configuration is loading, displays the spinner instead of the current page.
 */
export default function ConfigProvider({ children }: ConfigProviderProps) {
  const { isSuccess, isError, error, data } = useQuery<Config>({
    queryKey: ["config"],
    queryFn: async () => {
      const response = await fetch("/config/main.json");
      if (response.ok) {
        const config: unknown = await response.json();
        return Config.parse(config);
      }

      if (import.meta.env.DEV && response.status === 404) {
        throw new Error(
          "Add ./config/main.json file to the public folder in the repo and provide the application configuration.",
        );
      }
      throw new Error(
        `Failed to initialize the application: HTTP${response.status}.`,
      );
    },
  });

  if (isError) {
    return (
      <LoaderError title={"Failed to load configuration"}>
        {error instanceof ZodError ? (
          <>
            <Text fw={700} size={"sm"}>
              Invalid config.json:
            </Text>
            <List>
              {error.issues.map((issue) => {
                const path = issue.path.join(".");
                return (
                  <List.Item key={[issue.code, path].join()}>
                    <Text fw={700} size={"xs"}>
                      {path}:
                    </Text>
                    <Text size={"xs"}>{issue.message}</Text>
                  </List.Item>
                );
              })}
            </List>
          </>
        ) : (
          error.toString()
        )}
      </LoaderError>
    );
  }

  if (isSuccess) {
    return (
      <ConfigContext.Provider value={data}>{children}</ConfigContext.Provider>
    );
  } else {
    return <Loader m={"md"} />;
  }
}
