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

import ky from "ky";
import { CliError } from "@/output";
import { sleep } from "@/util/sleep";
import type {
  DiscoveryAuthClientInfo,
  DiscoveryAuthConfig,
} from "./discoveryService";

export interface Authentication {
  accessToken: string;
  idToken: string;
  refreshToken?: string;
  expiresAt: Date;
}

export interface TokenResponse {
  access_token: string;
  id_token: string;
  refresh_token?: string;
  expires_in: number;
}

interface OpenIdConfiguration {
  authorization_endpoint: string;
  token_endpoint: string;
  device_authorization_endpoint?: string;
}

interface DeviceAuthorizationResponse {
  device_code: string;
  user_code: string;
  verification_uri: string;
  expires_in: number;
  interval: number;
}

const CLIENT_NAME = "navigator";
let deviceFlowAllowed = true;

export function setDeviceFlowAllowed(allowed: boolean): void {
  deviceFlowAllowed = allowed;
}

export class AuthExpiredError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "AuthExpiredError";
  }
}

async function getOpenIdConfiguration(
  endpoint: string,
): Promise<OpenIdConfiguration> {
  return ky.get<OpenIdConfiguration>(endpoint).json();
}

function getClientInfo(config: DiscoveryAuthConfig): DiscoveryAuthClientInfo {
  const info = config.clients[CLIENT_NAME] ?? config.clients.default;
  if (!info) {
    throw new Error(`Client "${CLIENT_NAME}" not found in Discovery API.`);
  }
  return info;
}

function parseExpiresIn(expiresIn: number): Date {
  return new Date(Date.now() + expiresIn * 1000);
}

export async function refreshAuth(
  refreshToken: string,
  config: DiscoveryAuthConfig,
): Promise<Authentication> {
  const idp = await getOpenIdConfiguration(config.openidConfiguration);
  const clientInfo = getClientInfo(config);
  const form = new URLSearchParams();
  form.set("grant_type", "refresh_token");
  form.set("refresh_token", refreshToken);
  form.set("client_id", clientInfo.clientId);
  form.set("scope", clientInfo.scope);

  const response = await ky.post(idp.token_endpoint, {
    body: form.toString(),
    headers: { "Content-Type": "application/x-www-form-urlencoded" },
    throwHttpErrors: false,
  });

  if (response.status === 401) {
    throw new AuthExpiredError(`Session expired: HTTP401`);
  }
  if (response.status === 400) {
    const body = await response
      .json<{ error?: string }>()
      .catch((): { error?: string } => ({}));
    if (body.error === "invalid_grant") {
      throw new AuthExpiredError(`Session expired: ${body.error}`);
    }
    throw new Error(`Failed to refresh token: HTTP400 ${body.error ?? ""}`);
  }
  if (!response.ok) {
    throw new Error(`Failed to refresh token: HTTP${response.status}`);
  }

  const token = await response.json<TokenResponse>();
  return {
    accessToken: token.access_token,
    idToken: token.id_token,
    refreshToken: token.refresh_token,
    expiresAt: parseExpiresIn(token.expires_in),
  };
}

export async function deviceFlow(
  config: DiscoveryAuthConfig,
): Promise<Authentication> {
  if (!deviceFlowAllowed) {
    throw new CliError(
      "Authentication requires an interactive device login. Run 'sn host add' in a terminal.",
      "AUTH_REQUIRED",
    );
  }
  const idp = await getOpenIdConfiguration(config.openidConfiguration);
  if (!idp.device_authorization_endpoint) {
    throw new Error(
      "Identity provider does not support device authorization flow.",
    );
  }
  const clientInfo = getClientInfo(config);

  const form = new URLSearchParams();
  form.set("client_id", clientInfo.clientId);
  form.set("scope", clientInfo.scope);

  const deviceAuth = await ky
    .post<DeviceAuthorizationResponse>(idp.device_authorization_endpoint, {
      body: form.toString(),
      headers: { "Content-Type": "application/x-www-form-urlencoded" },
    })
    .json();

  process.stderr.write(
    `\nOpen this URL in your browser:\n  ${deviceAuth.verification_uri}\n`,
  );
  process.stderr.write(
    `Enter code: ${deviceAuth.user_code}\n\nWaiting for authorization...\n`,
  );

  return pollForToken(deviceAuth, idp, clientInfo);
}

async function pollForToken(
  deviceAuth: DeviceAuthorizationResponse,
  idp: OpenIdConfiguration,
  clientInfo: DiscoveryAuthClientInfo,
): Promise<Authentication> {
  let intervalMs = (deviceAuth.interval ?? 5) * 1000;
  const expiresAt = Date.now() + deviceAuth.expires_in * 1000;

  while (Date.now() < expiresAt) {
    await sleep(Math.min(intervalMs, expiresAt - Date.now()));
    if (Date.now() >= expiresAt) break;

    const form = new URLSearchParams();
    form.set("grant_type", "urn:ietf:params:oauth:grant-type:device_code");
    form.set("device_code", deviceAuth.device_code);
    form.set("client_id", clientInfo.clientId);

    const response = await ky.post(idp.token_endpoint, {
      body: form.toString(),
      headers: { "Content-Type": "application/x-www-form-urlencoded" },
      throwHttpErrors: false,
    });

    if (response.status === 200) {
      const token = await response.json<TokenResponse>();
      process.stderr.write("✓ Authenticated\n");
      return {
        accessToken: token.access_token,
        idToken: token.id_token,
        refreshToken: token.refresh_token,
        expiresAt: new Date(Date.now() + token.expires_in * 1000),
      };
    }

    const error = await response.json<{ error: string }>();
    if (error.error === "slow_down") {
      intervalMs += 5000;
      continue;
    }
    if (error.error === "authorization_pending") continue;
    throw new Error(`Device authorization failed: ${error.error}`);
  }

  throw new Error("Device authorization timed out.");
}
