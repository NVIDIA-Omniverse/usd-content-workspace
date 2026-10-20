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
import { Alert, Box, Button, Loader, Stack, Text } from "@mantine/core";
import { IconInfoCircle } from "@tabler/icons-react";
import { type ReactNode, useEffect, useState } from "react";
import { hasAuthParams, useAuth } from "react-oidc-context";
import { NavLink } from "react-router-dom";

export interface AuthRequiredProps {
  children?: ReactNode;
}

/**
 * Redirects the user to the identity provider if they are not authenticated.
 * All web pages in this example require authentication.
 */
export function AuthRequired({ children }: AuthRequiredProps) {
  const auth = useAuth();
  const [hasTriedSignIn, setHasTriedSignIn] = useState(false);

  useEffect(() => {
    if (
      !hasAuthParams() &&
      !auth.isAuthenticated &&
      !auth.activeNavigator &&
      !auth.isLoading &&
      !hasTriedSignIn
    ) {
      const handleError = (error: Error) => {
        console.error(error);
      };

      auth.events.addSilentRenewError(handleError);

      setHasTriedSignIn(true);
      void auth.signinSilent().then((user) => {
        if (user) {
          setHasTriedSignIn(false);
        } else {
          return auth.signinRedirect();
        }
      });

      return () => {
        auth.events.removeSilentRenewError(handleError);
      };
    }
  }, [auth, hasTriedSignIn]);

  if (auth.error && !auth.isLoading) {
    return (
      <Box p={"xl"}>
        <Alert
          variant="filled"
          color="red"
          title="Failed to authenticate"
          icon={<IconInfoCircle />}
        >
          <Stack align={"start"}>
            <Text size={"sm"}>{auth.error.message}</Text>
            <Button component={NavLink} to="/">
              Go back
            </Button>
          </Stack>
        </Alert>
      </Box>
    );
  }

  if (auth.isLoading || hasTriedSignIn) {
    return <Loader m={"md"} />;
  }

  if (!auth.isAuthenticated) {
    return null;
  }

  return children;
}
