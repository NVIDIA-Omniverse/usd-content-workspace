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

import { jwtDecode } from "jwt-decode";
import ky, { HTTPError } from "ky";
import { z } from "zod";
import { fromSnakeCaseSchema } from "@/util/schemas";
import { ApiOptions, create, isHttpError } from "./api";
import { resolveDiscoveryUrls } from "./discoveryService";

export const CLIENT_NAME = "navigator";

const DiscoveryAuthClientInfoSchema = fromSnakeCaseSchema(
  z.object({
    clientId: z.string(),
    scope: z.string(),
  })
);

export type DiscoveryAuthClientInfo = z.infer<typeof DiscoveryAuthClientInfoSchema>;

export const DiscoveryAuthConfigSchema = fromSnakeCaseSchema(
  z.object({
    openidConfiguration: z.string(),
    clients: z.record(z.string(), DiscoveryAuthClientInfoSchema),
  })
);

export type DiscoveryAuthConfig = z.infer<typeof DiscoveryAuthConfigSchema>;

/**
 * Retrieves the auth configuration from the discovery service, or `null` when the deployment
 * runs without authentication (the endpoint responds with 404).
 */
export async function getAuthConfig(hostname: string, options?: ApiOptions): Promise<DiscoveryAuthConfig | null> {
  const { v1Url } = resolveDiscoveryUrls(hostname);
  const api = create(v1Url, options);
  try {
    const response = await api.get("auth-config");
    return DiscoveryAuthConfigSchema.parse(await response.json());
  } catch (error) {
    if (isHttpError(error) && error.response.status === 404) {
      return null;
    }
    throw error;
  }
}

export interface Authentication {
  /**
   * Token that can be passed to other applications and services for authenticating the user.
   */
  accessToken: string;

  /**
   * JWT that can be parsed by the application to display user information.
   * Must not be passed to other applications.
   */
  idToken: string;

  /**
   * Token that can be used to refresh the current session.
   * The identity provider must be configured with Refresh Token Rotation, otherwise considered not secure.
   */
  refreshToken?: string;

  /**
   * Time when access and id tokens expire.
   */
  expiresAt: Date;
}

/**
 * Defines information saved in a redirect to the identity provider.
 * This information is restored when the user is redirected back to the application after entering credentials.
 */
export interface AuthenticationState {
  urls: {
    discovery: string;
  };
}

/**
 * Claims that the application reads from the id token to display user information.
 */
export interface IdTokenClaims {
  email?: string;
  [claim: string]: unknown;
}

/**
 * Returns the email of the currently authenticated user by parsing the id token,
 * or `undefined` when the token is missing the email claim or cannot be parsed.
 *
 * @param idToken JWT received from the identity provider.
 */
export function getUserEmail(idToken: string): string | undefined {
  const email = decodeIdToken(idToken)?.email;
  return typeof email === "string" ? email : undefined;
}

/**
 * Decodes the payload of a JWT id token without verifying its signature.
 * Signature verification is the identity provider's responsibility; the application
 * only reads claims to display user information.
 *
 * @param idToken JWT received from the identity provider.
 */
function decodeIdToken(idToken: string): IdTokenClaims | undefined {
  try {
    return jwtDecode<IdTokenClaims>(idToken);
  } catch {
    return undefined;
  }
}

/**
 * Constructs the authorize URL for OpenID Connect and redirects the user to the identity provider (IdP).
 * Retrieves IdP information via OpenID Connect Discovery and
 * starts user authentication with authorization flow with PKCE.
 *
 * @param state Application state that needs to be remembered between redirects.
 *    Must contain OpenID Connect Discovery URL and Storage API Discovery URL.
 *    See {@link https://openid.net/specs/openid-connect-discovery-1_0.html#ProviderConfigurationResponse} for more info on OpenID Connect Discovery URL.
 * @param config Configuration received from the Discovery API.
 */
export async function authorize(state: AuthenticationState, config: DiscoveryAuthConfig): Promise<void> {
  const idp = await getOpenIdConfiguration(config.openidConfiguration);
  const params = new URLSearchParams();
  params.set("response_type", "code");

  const info = getClientInfo(config);
  params.set("client_id", info.clientId);
  params.set("scope", info.scope);

  params.set("nonce", crypto.randomUUID());
  params.set("redirect_uri", getRedirectUri());

  const [codeVerifier, codeChallenge] = await generateCodeChallenge();
  params.set("code_challenge", codeChallenge);
  params.set("code_challenge_method", "S256");
  sessionStorage.setItem("codeVerifier", codeVerifier);

  params.set("state", encodeState(state));
  window.location.href = `${idp.authorization_endpoint}?${params}`;
}

/**
 * Logs the user out by redirecting to the identity provider's RP-initiated logout endpoint,
 * ending the session at the IdP and returning to the application afterwards.
 *
 * When the IdP does not advertise an `end_session_endpoint`, the local tokens are the only
 * session state, so the user is simply sent back to the application root.
 *
 * @param config Configuration received from the Discovery API.
 * @param idToken Current id token, passed as `id_token_hint` so the IdP can identify the session.
 */
export async function logout(config: DiscoveryAuthConfig, idToken?: string): Promise<void> {
  const idp = await getOpenIdConfiguration(config.openidConfiguration);
  if (!idp.end_session_endpoint) {
    window.location.href = getPostLogoutRedirectUri();
    return;
  }

  const params = new URLSearchParams();
  if (idToken) {
    params.set("id_token_hint", idToken);
  }
  params.set("post_logout_redirect_uri", getPostLogoutRedirectUri());

  const info = getClientInfo(config);
  params.set("client_id", info.clientId);

  window.location.href = `${idp.end_session_endpoint}?${params}`;
}

export interface OpenIdConfiguration {
  authorization_endpoint: string;
  token_endpoint: string;
  userinfo_endpoint: string;
  scopes_supported: string[];
  end_session_endpoint?: string;
}

/**
 * Gets OpenID Connect configuration from the IdP by using OpenID Connect Discovery.
 * See {@link https://openid.net/specs/openid-connect-discovery-1_0.html#ProviderConfigurationResponse} for more info.
 *
 * @param endpoint OpenID Connect Discovery URL.
 */
async function getOpenIdConfiguration(endpoint: string): Promise<OpenIdConfiguration> {
  return ky.get<OpenIdConfiguration>(endpoint, { cache: "force-cache" }).json();
}

/**
 * Returns the authentication info such as clientId and scopes
 * from the configuration received from the Discovery API.
 * @param config
 */
function getClientInfo(config: DiscoveryAuthConfig): DiscoveryAuthClientInfo {
  let clientInfo = config.clients[CLIENT_NAME];
  if (!clientInfo) {
    clientInfo = config.clients.default;
  }
  if (!clientInfo) {
    throw new Error(`Client "${CLIENT_NAME}" or the default client is not found in Discovery API.`);
  }
  return clientInfo;
}

/**
 * Returns a URL that should be used by the IdP to redirect the user back to the application.
 */
function getRedirectUri(): string {
  return `${window.location.protocol}//${window.location.host}/openid`;
}

/**
 * Returns the URL the IdP should redirect the user back to after RP-initiated logout.
 */
function getPostLogoutRedirectUri(): string {
  return `${window.location.protocol}//${window.location.host}/`;
}

/**
 * Generates code verifier and code challenge for authorization flow with PKCE.
 * Code verifier is cached in the browser tab to be used after the redirect from the IdP.
 * Code challenge is sent to the IdP for generating code as the auth result.
 */
async function generateCodeChallenge(): Promise<readonly [string, string]> {
  // Utility to base64 encode a sequence of octets (URL-safe alphabet, no padding)
  function b64url(array: Uint8Array) {
    const b64 = btoa(String.fromCharCode.apply(null, [...array]));
    return b64.replace(/\+/g, "-").replace(/\//g, "_").replace(/=/g, "");
  }

  // Generate a 32-octet random sequence
  const rand = crypto.getRandomValues(new Uint8Array(32));

  // The PKCE code verifier is the base64url encoding of the random octets
  const codeVerifier = b64url(rand);

  // Calculate the SHA-256 hash of the code verifier
  const data = new TextEncoder().encode(codeVerifier);
  const sha256 = await crypto.subtle.digest("SHA-256", data);

  // The PKCE code challenge is the base64url encoding of the hash
  const codeChallenge = b64url(new Uint8Array(sha256));

  return [codeVerifier, codeChallenge];
}

/**
 * Serializes the state that should be passed to the IdP and restored when the user will be redirected
 * back to the application.
 * @param state
 */
export function encodeState(state: AuthenticationState): string {
  return btoa(JSON.stringify(state));
}

/**
 * Deserialized the state that was saved before user was redirected to the IdP.
 * @param state
 */
export function decodeState(state: string): AuthenticationState {
  return JSON.parse(atob(state)) as AuthenticationState;
}

/**
 * Uses authorization flow with PKCE to exchange code and state received from the IdP after the redirect
 * and get authentication tokens that can be used in the application.
 *
 * @param code Code received from the query parameter.
 * @param config Configuration received from the Discovery API.
 */
export async function exchangeCode(code: string, config: DiscoveryAuthConfig): Promise<Authentication> {
  const codeVerifier = sessionStorage.getItem("codeVerifier");
  if (!codeVerifier) {
    throw new Error("Login results are expired. Please try to authenticate again.");
  }

  // Remove the cached verifier since it must be used only once.
  sessionStorage.removeItem("codeVerifier");

  // Exchange received code to auth tokens
  const idp = await getOpenIdConfiguration(config.openidConfiguration);
  const form = new URLSearchParams();
  form.set("grant_type", "authorization_code");
  form.set("code", code);
  form.set("redirect_uri", getRedirectUri());
  form.set("code_verifier", codeVerifier);

  const info = getClientInfo(config);
  form.set("client_id", info.clientId);
  try {
    const response = await ky
      .post<TokenResponse>(idp.token_endpoint, {
        body: form.toString(),
        headers: {
          "Content-Type": "application/x-www-form-urlencoded",
        },
      })
      .json();

    return {
      accessToken: response.access_token,
      idToken: response.id_token,
      refreshToken: response.refresh_token,
      expiresAt: parseExpiresIn(response.expires_in),
    };
  } catch (error) {
    if ((error as Error).name === "HTTPError") {
      const httpError = error as HTTPError;
      const wwwAuthenticate = httpError.response.headers.get("WWW-Authenticate");
      throw new Error(`Failed to authenticate: HTTP${httpError.response.status}. ${wwwAuthenticate}`);
    }

    // Unknown error.
    throw error;
  }
}

/**
 * Converts relative time received from `expires_in` field from TokenResponse to a Date instance.
 * @param expiresIn
 */
function parseExpiresIn(expiresIn: number): Date {
  return new Date(Date.now() + expiresIn * 1000);
}

export interface TokenResponse {
  access_token: string;
  id_token: string;
  refresh_token?: string;
  expires_in: number;
}

/**
 * Refreshes the user session using the specified refresh token and OpenID Connect Discovery URL.
 * @param refreshToken Refresh token obtained after authentication or previous session refresh.
 * @param config The configuration received from the Discovery API.
 */
export const refreshAuth = async (refreshToken: string, config: DiscoveryAuthConfig): Promise<Authentication> => {
  const idp = await getOpenIdConfiguration(config.openidConfiguration);
  const form = new URLSearchParams();
  form.set("grant_type", "refresh_token");
  form.set("refresh_token", refreshToken);

  const info = getClientInfo(config);
  form.set("client_id", info.clientId);
  form.set("scope", info.scope);
  try {
    const response = await ky
      .post<TokenResponse>(idp.token_endpoint, {
        body: form.toString(),
        headers: {
          "Content-Type": "application/x-www-form-urlencoded",
        },
      })
      .json();

    return {
      accessToken: response.access_token,
      idToken: response.id_token,
      refreshToken: response.refresh_token,
      expiresAt: parseExpiresIn(response.expires_in),
    };
  } catch (error) {
    if (isHttpError(error)) {
      const wwwAuthenticate = error.response.headers.get("WWW-Authenticate");
      const message = `Failed to refresh the session: HTTP${error.response.status}. ${wwwAuthenticate}`;
      if (error.response.status === 401) {
        throw new AuthExpiredError(message);
      }
      throw new Error(message);
    }

    // Unknown error.
    throw error;
  }
};

export class AuthExpiredError extends Error {}
