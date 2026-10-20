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
import { Log, UserManager, WebStorageStateStore } from "oidc-client-ts";
import {
  type ReactNode,
  useCallback,
  useEffect,
  useMemo,
  useState,
} from "react";
import {
  AuthProvider as OIDCProvider,
  type AuthProviderProps as OIDCProviderProps,
} from "react-oidc-context";
import { useNavigate, useSearchParams } from "react-router-dom";
import { useConfig } from "../hooks/useConfig";

export interface AuthProviderProps {
  children?: ReactNode;
}

Log.setLogger(console);
Log.setLevel(Log.DEBUG);

interface AuthMessage {
  type: "logout";
}

/**
 * Integrates OpenID Connect to the portal and provides corresponding authentication information as context.
 * Stores authentication tokens in cookies.
 */
export function AuthProvider({ children }: AuthProviderProps) {
  const [, setSearchParams] = useSearchParams();
  const navigate = useNavigate();
  const [channel] = useState(() => new BroadcastChannel("session"));

  const onSignIn = useCallback(() => {
    setSearchParams({});
  }, [setSearchParams]);

  const onRemoveUser = useCallback(() => {
    channel.postMessage({ type: "logout" });
    navigate("/");
  }, [channel, navigate]);

  const config = useConfig();
  const auth: OIDCProviderProps = useMemo(
    () => ({
      userManager: new UserManager({
        authority: config.auth.authority,
        client_id: config.auth.clientId,
        redirect_uri: config.auth.redirectUri,
        scope: config.auth.scope ?? "openid profile email",
        refreshTokenAllowedScope: config.auth.scope ?? "openid profile email",

        userStore: new WebStorageStateStore({
          store: window.localStorage,
        }),
      }),
    }),
    [config],
  );

  useEffect(() => {
    const listener = (event: MessageEvent) => {
      const message = event.data as AuthMessage;
      if (message.type === "logout") {
        void auth.userManager?.removeUser();
      }
    };

    channel.addEventListener("message", listener);
    return () => channel.removeEventListener("message", listener);
  }, [channel, auth]);

  return (
    <OIDCProvider
      {...auth}
      onSigninCallback={onSignIn}
      onRemoveUser={onRemoveUser}
    >
      {children}
    </OIDCProvider>
  );
}
