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

import React, { Component, Suspense } from "react";
import { IconAlertTriangle } from "@tabler/icons-react";
import { Alert, Center, Code, List, Stack, Text } from "@mantine/core";
import { PageLoader } from "@/components/PageLoader";
import { useAppConfig } from "@/hooks/useAppConfig";
import { AppConfigError } from "@/services/appConfigService";

export interface AppConfigProviderProps {
  children?: React.ReactNode;
}

/**
 * Suspends rendering of the rest of the app until the runtime `config.json`
 * has been fetched and parsed. Children may then call `useAppConfig()` synchronously.
 * A configuration that fails to parse or validate is surfaced as a blocking error screen.
 */
export function AppConfigProvider({ children }: AppConfigProviderProps) {
  return (
    <AppConfigErrorBoundary>
      <Suspense fallback={<PageLoader label="Loading application configuration" />}>
        <AppConfigGate>{children}</AppConfigGate>
      </Suspense>
    </AppConfigErrorBoundary>
  );
}

interface AppConfigErrorBoundaryProps {
  children?: React.ReactNode;
}

interface AppConfigErrorBoundaryState {
  error: Error | null;
}

class AppConfigErrorBoundary extends Component<AppConfigErrorBoundaryProps, AppConfigErrorBoundaryState> {
  state: AppConfigErrorBoundaryState = { error: null };

  static getDerivedStateFromError(error: Error): AppConfigErrorBoundaryState {
    return { error };
  }

  render() {
    const { error } = this.state;
    if (error) {
      if (error instanceof AppConfigError) {
        return <AppConfigErrorScreen error={error} />;
      }
      throw error;
    }
    return this.props.children;
  }
}

function AppConfigGate({ children }: { children?: React.ReactNode }) {
  useAppConfig();
  return <>{children}</>;
}

function AppConfigErrorScreen({ error }: { error: AppConfigError }) {
  const issues = error.issues;

  return (
    <Center h="100vh" w="100vw" p="md">
      <Alert
        variant="light"
        color="red"
        icon={<IconAlertTriangle />}
        title="Invalid application configuration"
        maw={640}
      >
        <Stack gap="sm">
          <Text size="sm">
            The runtime configuration (config.json) could not be loaded. Please review the deployment configuration.
          </Text>
          <Text size="sm" fw={500}>
            {error.message}
          </Text>
          {issues.length > 0 && (
            <List size="sm" spacing={4}>
              {issues.map((issue) => (
                <List.Item key={issue}>
                  <Code>{issue}</Code>
                </List.Item>
              ))}
            </List>
          )}
        </Stack>
      </Alert>
    </Center>
  );
}
