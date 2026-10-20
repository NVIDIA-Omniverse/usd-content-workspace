# Microsoft Graph User Info Service Smoke Test

A smoke test binary for validating a deployed
`microsoft-graph-userinfo-service`. The test exercises the public service API
end-to-end over **both REST and gRPC** through the same ingress that production
traffic uses, and is designed to be run as a Helm test hook after a chart
install / upgrade.

Tests execute sequentially and **stop after the first failure**. Each failure
is printed with a detailed error message and a remediation hint that lists the
Helm values and Kubernetes objects to inspect.

## Authentication model

The smoke test acquires **two independent bearer tokens** because the user
service deployment commonly sits behind a Contour `authExtension` whose
expected audience is the user service's own app registration — a Microsoft
Graph-scoped Entra token would be rejected at the ingress.

- **Service bearer** *(optional)* — used for every call into the user service
  (`/healthz`, `/readyz`, `/api/v1alpha/*`, gRPC) when configured. Acquired
  via the OAuth2 client-credentials flow driven by OpenID Connect Discovery,
  matching the conformance test's `serviceIdentity.clientCredentials` Helm
  block. The scope passed to the identity provider must yield a token whose
  `aud` matches the auth extension's expected audience (typically
  `<service-app-id>/.default`). When no service-bearer credentials are
  configured the smoke test runs **unauthenticated**, which matches
  deployments without an auth extension or without an HTTPProxy at all.
- **Microsoft Graph bearer** *(optional)* — used solely by the optional
  Microsoft Graph step (Test 3). Acquired through Microsoft Entra ID
  directly, scoped for Microsoft Graph (`https://graph.microsoft.com/.default`).

## Test cases

1. **Acquire user service bearer** *(skipped when not configured)* — runs the
   OAuth2 client-credentials flow against the configured OpenID Connect
   Discovery URI and uses the resulting access token for every subsequent
   call into the user service. Pre-flight waiting and `/healthz` / `/readyz`
   probes both send this bearer. When no service-bearer credentials are
   configured (Helm: `serviceIdentity.clientCredentials.enabled=false`,
   no `--bearer-token` / `--access-token-file` / OAuth2 flags), this test is
   skipped (not recorded as a pass or a fail) and all subsequent calls run
   without an `Authorization` header.
2. **Service health** — `GET /healthz` and `GET /readyz` confirm the service
   is up and the initial directory sync has completed. Both calls carry the
   service bearer when one was acquired in Test 1, otherwise they run
   unauthenticated.
3. **Call Microsoft Graph API** *(optional)* — when `AZURE_TENANT_ID` and
   `AZURE_CLIENT_ID` are set, acquires a Microsoft Graph-scoped Entra token
   (supporting `client-secret` and `workload-identity` modes) and issues
   `GET https://graph.microsoft.com/v1.0/users?$top=1` to verify the smoke
   test pod's Entra credentials are accepted by Graph and that the app
   registration has the required application permissions. Skipped when the
   `AZURE_*` variables are not configured.
4. **Public User Info REST API**
   - `GET /api/v1alpha/users?limit=N` — one page of users.
   - `GET /api/v1alpha/users/{userId}` — single user (using the first ID from
     the list).
   - `GET /api/v1alpha/groups?limit=N` — one page of groups.
   - `GET /api/v1alpha/groups/{groupId}` — single group (using the first ID
     from the list).
   - `GET /api/v1alpha/users/{userId}/groups` — group membership for the user
     picked from the list.
5. **Public User Info gRPC API** (enabled by default; mirrors the REST cases)
   - `grpc.health.v1.Health/Check` — confirms the gRPC port is reachable and
     the service is `SERVING`.
   - `UserInfoService.ListUsers`
   - `UserInfoService.GetUser`
   - `UserInfoService.ListGroups`
   - `UserInfoService.GetGroup`
   - `UserInfoService.ListUserGroups`

## Command-line arguments

Every argument can also be passed as the corresponding environment variable.

### Service URL and transport

| Argument                        | Environment variable           | Required                   | Description                                                                          |
| ------------------------------- | ------------------------------ | -------------------------- | ------------------------------------------------------------------------------------ |
| `--rest-url`                    | `REST_URL`                     | Yes                        | Base URL of the REST API (e.g. `https://userinfo.example.com`). `SERVICE_URL` is accepted as a back-compat alias. |
| `--grpc-enabled`                | `GRPC_ENABLED`                 | No (default `true`)        | Whether to run the gRPC test suite after REST tests.                                 |
| `--grpc-endpoint`               | `GRPC_ENDPOINT`                | No                         | gRPC endpoint as `host:port` or `scheme://host:port`. Defaults to `<rest-host>:<grpc-port>`. |
| `--grpc-port`                   | `GRPC_PORT`                    | No (default `50051`)       | Port used when deriving a default gRPC endpoint from the REST URL.                   |
| `--grpc-tls-enabled`            | `GRPC_TLS_ENABLED`             | No (default `false`)       | Connect to the gRPC endpoint over TLS (native-root trust store).                     |

### User-service bearer (Test 1)

Optional. When none of the sources below are configured, Test 1 is skipped
and the smoke test runs unauthenticated against the service — matching
deployments that do not enable an auth extension. When at least one source is
configured, the resolution order is: literal `--bearer-token`,
`--access-token-file`, OAuth2 client-credentials.

| Argument                              | Environment variable                  | Description                                                                          |
| ------------------------------------- | ------------------------------------- | ------------------------------------------------------------------------------------ |
| `--bearer-token`                      | `BEARER_TOKEN`                        | Pre-acquired bearer literal (local development).                                     |
| `--access-token-file`                 | `ACCESS_TOKEN_FILE`                   | Path to a JSON token file populated by a token-provisioner sidecar.                  |
| `--oauth2-openid-configuration-uri`   | `OAUTH2_OPENID_CONFIGURATION_URI`     | OpenID Connect Discovery URI; the smoke test reads `token_endpoint` from the metadata. |
| `--oauth2-client-id`                  | `OAUTH2_CLIENT_ID`                    | Client ID for the client-credentials flow.                                           |
| `--oauth2-client-secret`              | `OAUTH2_CLIENT_SECRET`                | Client secret literal.                                                               |
| `--oauth2-client-secret-file`         | `OAUTH2_CLIENT_SECRET_FILE`           | Path to a file containing the client secret.                                         |
| `--oauth2-client-scope`               | `OAUTH2_CLIENT_SCOPE`                 | `scope` value passed to the identity provider, e.g. `<service-app-id>/.default`.     |
| `--oauth2-extra-headers`              | `OAUTH2_EXTRA_HEADERS`                | Extra HTTP headers for the token-exchange request, JSON object (default `{}`).       |
| `--oauth2-extra-body-params`          | `OAUTH2_EXTRA_BODY_PARAMS`            | Extra body parameters for the token-exchange request, JSON object (default `{}`).    |

### Microsoft Graph bearer (Test 3, optional)

| Argument                        | Environment variable           | Required                   | Description                                                                          |
| ------------------------------- | ------------------------------ | -------------------------- | ------------------------------------------------------------------------------------ |
| `--azure-tenant-id`             | `AZURE_TENANT_ID`              | Required to run Test 3     | Entra ID tenant ID.                                                                  |
| `--azure-client-id`             | `AZURE_CLIENT_ID`              | Required to run Test 3     | App registration client ID.                                                          |
| `--azure-auth-mode`             | `AZURE_AUTH_MODE`              | No (default `client-secret`) | `client-secret` or `workload-identity`.                                              |
| `--azure-client-secret`         | `AZURE_CLIENT_SECRET`          | When authMode=client-secret* | Client secret literal.                                                               |
| `--azure-client-secret-file`    | `AZURE_CLIENT_SECRET_FILE`     | When authMode=client-secret* | Path to a file containing the client secret.                                         |
| `--azure-federated-token-file`  | `AZURE_FEDERATED_TOKEN_FILE`   | No                         | Projected SA token path (for workload-identity).                                     |
| `--graph-base-url`              | `GRAPH_BASE_URL`               | No                         | Microsoft Graph base URL (default `https://graph.microsoft.com/v1.0`).               |
| `--graph-scope`                 | `GRAPH_SCOPE`                  | No                         | OAuth2 scope (default `https://graph.microsoft.com/.default`).                       |

Test 3 is automatically skipped when both `AZURE_TENANT_ID` and `AZURE_CLIENT_ID` are unset or empty.

\* Either `AZURE_CLIENT_SECRET` or `AZURE_CLIENT_SECRET_FILE` is required in
`client-secret` mode.

### Smoke-test runtime

| Argument                        | Environment variable           | Required                   | Description                                                                          |
| ------------------------------- | ------------------------------ | -------------------------- | ------------------------------------------------------------------------------------ |
| `--wait-retries`                | `WAIT_RETRIES`                 | No                         | Readiness retry count (default `10`).                                                |
| `--wait-interval`               | `WAIT_INTERVAL`                | No                         | Seconds between readiness retries (default `5`).                                     |
| `--page-limit`                  | `PAGE_LIMIT`                   | No                         | `limit` query parameter for list endpoints (default `10`).                           |
| `--http-timeout`                | `HTTP_TIMEOUT`                 | No                         | HTTP request timeout in seconds (default `30`).                                      |
| `--helm`                        | `HELM`                         | No                         | Render Helm-specific hints in error messages.                                        |

## Running standalone

```bash
cargo run --package microsoft-graph-userinfo-smoke-test -- \
  --rest-url https://userinfo.example.com \
  --grpc-endpoint userinfo.example.com:50051 \
  --oauth2-openid-configuration-uri "https://login.microsoftonline.com/$AZURE_TENANT_ID/v2.0/.well-known/openid-configuration" \
  --oauth2-client-id "$OAUTH2_CLIENT_ID" \
  --oauth2-client-secret "$OAUTH2_CLIENT_SECRET" \
  --oauth2-client-scope "$OAUTH2_CLIENT_ID/.default" \
  --azure-tenant-id "$AZURE_TENANT_ID" \
  --azure-client-id "$AZURE_CLIENT_ID" \
  --azure-client-secret "$AZURE_CLIENT_SECRET"
```

## Running in Kubernetes via Helm

The smoke test is packaged as a Helm test hook. After installing the chart,
trigger it with:

```bash
helm test <release-name> --logs
```

Behavior and configuration are controlled by the `smokeTest.*` values in
`helm/microsoft-graph-userinfo-service/values.yaml`.

## Exit codes

| Code | Meaning             |
| ---- | ------------------- |
| 0    | All tests passed    |
| 1    | A test failed or    |
|      | the service did not |
|      | become reachable    |
