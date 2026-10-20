(permission-service-authentication-authenticating-with-an-openid-connect-identity-provider)=
# Authenticating with an OpenID Connect Identity Provider

Permission Service does not mint credentials of its own. Every authenticated call into the service — and every authenticated call the service makes to other Omniverse services — is backed by a bearer token issued by a separate **OpenID Connect (OIDC)** identity provider. This page documents the two sides of that integration as it is exposed through the Helm chart:

- The top-level **`openId`** section, which configures how the service validates bearer tokens presented by **incoming** REST and gRPC requests.
- The top-level **`serviceIdentity`** section, which configures how the service obtains a bearer token for its own **outgoing** calls to other services (for example, the Notification Service — see {ref}`Service notifications <permission-service-notifications-service-notifications>`).

For how the validated principal is used during policy evaluation, see {ref}`Identity Verification <permission-service-policy-evaluation-1-identity-verification>` and {ref}`Principal <permission-service-policy-language-principal>`. For the administrative actions that only authenticated callers can reach, see {ref}`Granting administrative permissions <permission-service-deployment-granting-administrative-permissions>`.

(permission-service-authentication-configuration)=
## Configuration

Inbound authentication is **enabled by default**; outbound service identity is **disabled by default**, so a default deployment verifies the tokens it receives and calls other services without presenting one. The settings that decide each side:

| Value                                            | Default  | Purpose                                                                                                                                      |
|--------------------------------------------------|----------|----------------------------------------------------------------------------------------------------------------------------------------------|
| `openId.enabled`                                 | `true`   | Master switch for verifying tokens on incoming REST and gRPC requests. When `false`, every request is accepted as an anonymous principal.      |
| `openId.openIdConfigurationUri`                  | `""`     | Discovery document URL of the identity provider. Required when `openId.enabled` is `true`; startup fails without a reachable document.        |
| `openId.tokenVerificationType`                   | `jwt`    | How presented tokens are verified: `jwt`, `opaque`, or `jwtuserinfo`. See {ref}`Token verification types <permission-service-authentication-token-verification-types>`. |
| `serviceIdentity.clientCredentials.enabled`      | `false`  | Obtains outbound tokens through the OAuth 2.0 client-credentials grant. See {ref}`Client credentials flow <permission-service-authentication-client-credentials-flow>`. |
| `serviceIdentity.accessTokenFile.enabled`        | `false`  | Reads outbound tokens from a file written by a sidecar. See {ref}`Access-token file <permission-service-authentication-access-token-file>`.     |

At most one of the two `serviceIdentity` modes may be enabled. The full value reference for each section is documented under {ref}`Helm values <permission-service-authentication-helm-values>`, {ref}`Client credentials flow <permission-service-authentication-client-credentials-flow>`, and {ref}`Access-token file <permission-service-authentication-access-token-file>` below.

(permission-service-authentication-configuring-authentication-in-the-helm-chart)=
### Configuring authentication in the Helm Chart

A deployment that verifies incoming tokens and authenticates its own outgoing calls with the client-credentials grant sets both sections:

```yaml
openId:
  enabled: true
  tokenVerificationType: "jwt"
  openIdConfigurationUri: "https://idp.example.com/.well-known/openid-configuration"
  allowedJwtAudience:
    - "permission-service-web"

serviceIdentity:
  clientCredentials:
    enabled: true
    clientId: "permission-service"
    clientScope: "notifications.publish"
    clientSecretRef:
      name: permission-service-client-credentials
      key: clientSecret
```

The client secret is supplied out of band as an existing Kubernetes secret; the chart mounts it and never renders its value into a ConfigMap. Both sections are read at startup and reach the container as environment variables, so changing any of these values rolls the pods on the next upgrade.

(permission-service-authentication-incoming-request-authentication-openid)=
## Incoming request authentication (`openId`)

The Helm chart exposes a top-level `openId` section whose values drive everything in this chapter; in particular, the Helm value **`openId.enabled`** is the master switch that turns inbound token verification on or off for the deployed service. The full list of values is documented under {ref}`Helm values <permission-service-authentication-helm-values>` below.

When `openId.enabled` is `true`, every REST API request and every `PermissionService` gRPC call must carry an `Authorization: Bearer <token>` header. Requests without a bearer token, or with a token the configured identity provider rejects, are answered with `401 Unauthorized` (gRPC `UNAUTHENTICATED`) before any Cedar policy is evaluated.

The following surfaces do not require a token:

- `GET /health` and `GET /v1beta/idp/keys/` on the REST port are public.
- The Swagger UI and `GET /openapi.json` are served outside the authentication layer.
- On the gRPC port, only `PermissionService` calls are authenticated; the gRPC health, server reflection, and capabilities services are reachable without a token.

When `openId.enabled` is `false`, the service accepts every request without inspecting the header and treats the caller as an anonymous principal. This mode is only intended for local development and testing; {ref}`meta-permissions <permission-service-deployment-granting-administrative-permissions>` are also not enforced in that mode, which means every endpoint is reachable without credentials.

(permission-service-authentication-helm-values)=
### Helm values

The chart exposes authentication settings under the top-level `openId` key:

```yaml
openId:
  enabled: true
  tokenVerificationType: "jwt"
  openIdConfigurationUri: "https://idp.example.com/.well-known/openid-configuration"
  allowedJwtAudience:
    - "permission-service-web"
  allowedJwtIssuers: []
  requiredJwtClaims: []
  principalIdClaim: "sub"
  principalGroupsClaim: "groups"
```

| Value                              | Purpose |
|------------------------------------|---------|
| `openId.enabled`                   | Master switch. When `false`, bearer tokens are not inspected and every request is anonymous. Default `true`. |
| `openId.tokenVerificationType`     | Selects the verification strategy — `jwt`, `opaque`, or `jwtuserinfo`. See {ref}`Token verification types <permission-service-authentication-token-verification-types>`. Default `jwt`. |
| `openId.openIdConfigurationUri`    | URL of the provider's OIDC discovery document, typically ending in `/.well-known/openid-configuration`. Required when `openId.enabled` is `true`. |
| `openId.allowedJwtAudience`        | List of accepted values for the JWT `aud` claim — typically the client id of each calling application, or an API identifier when the IdP stamps that into `aud` instead (for example, Microsoft Entra ID — see {ref}`below <permission-service-authentication-configuring-microsoft-entra-id>`). Default `[]`. Required when `openId.enabled` is `true` and `openId.tokenVerificationType` is `jwt` or `jwtuserinfo`; not used with `opaque`. |
| `openId.allowedJwtIssuers`         | List of accepted values for the JWT `iss` claim. The discovery-document issuer is not added implicitly, so every trusted issuer must be listed. Default `[]` (the `iss` claim is not checked). Not used with `opaque`. |
| `openId.requiredJwtClaims`         | Rules that the claims of every verified token must satisfy in addition to the `aud` and `iss` checks — for example, that the calling client id is one of an allowed set. Default `[]` (no extra checks). Maps to the `JWT_REQUIRED_CLAIMS` environment variable. See {ref}`Required claims <permission-service-authentication-required-claims>`. |
| `openId.principalIdClaim`          | The JWT claim used as the **principal id** for incoming callers. Defaults to `sub`. Per-service overrides are still possible through service metadata; see {ref}`idClaim for principals <permission-service-metadata-idclaim-for-principals>`. |
| `openId.principalGroupsClaim`      | The JWT claim holding the caller's **group memberships**, used to evaluate `principal in Group::"<id>"` policy scopes. Must be a JSON array of group id strings. Defaults to `groups`. Maps to the `PRINCIPAL_GROUPS_CLAIM` environment variable. See {ref}`Groups and membership <permission-service-policy-language-groups-and-membership>`. |

The chart sorts and deduplicates `openId.allowedJwtAudience` and passes it to the service as the `JWT_AUDIENCE` environment variable. Tokens are rejected when their `aud` claim is missing, is neither a string nor a list of strings, or matches no entry on that list. When the list is empty, the chart omits `JWT_AUDIENCE`; with `openId.enabled: true` and `tokenVerificationType` set to `jwt` or `jwtuserinfo`, the service then logs `Failed to initialize authentication: JWT audience is not specified` and exits at startup, so the pod stays in `CrashLoopBackOff` until an audience is configured. With `opaque`, tokens carry no `aud` claim to check, so the list may be left empty and is ignored when set.

`openId.allowedJwtIssuers` is sorted, deduplicated, and passed as the `JWT_ISSUERS` environment variable in the same way. When the list is set, tokens are rejected when their `iss` claim is missing, is neither a string nor a list of strings, or matches no entry on that list. When the list is empty, the chart omits `JWT_ISSUERS` and the `iss` claim is not checked.

(permission-service-authentication-required-claims)=
### Required claims

`openId.requiredJwtClaims` restricts which verified tokens are accepted, based on any top-level claim of the token — for example, only tokens issued to specific client applications, or only callers that belong to one of a set of groups. The list is empty by default, so a default deployment applies no checks beyond the signature, `exp`, `nbf`, `aud`, and `iss` validation described in this chapter.

Each rule names a `claim`, an `operator`, and — depending on the operator — a single `value` or a `values` list. Every rule must pass; a token that fails any rule is rejected with `401 Unauthorized` (gRPC `UNAUTHENTICATED`) and, like a token with an invalid signature, is recorded in the negative cache for `config.invalidTokenCacheTtl`, so retries with it are rejected without verifying it again — in `opaque` mode, without another `userinfo` call.

| Operator      | Operand  | Passes when |
|---------------|----------|-------------|
| `equals`      | `value`  | The claim holds a single value equal to `value`. |
| `notEquals`   | `value`  | The claim holds a single value different from `value`. |
| `in`          | `values` | The claim holds a single value that is one of `values`. |
| `notIn`       | `values` | The claim holds a single value that is none of `values`. |
| `contains`    | `value`  | The claim is an array that includes `value`. |
| `containsAny` | `values` | The claim is an array that includes at least one item of `values`. |
| `containsAll` | `values` | The claim is an array that includes every item of `values`; extra elements are allowed. |
| `exists`      | none     | The claim is present and not `null`. |

Values are strings, integers, or booleans, and are compared with the claim strictly by type and case: the string `"1"` never matches the integer `1`, `"Admins"` never matches `"admins"`, and a claim holding a decimal number never matches an integer value.

How claims of different shapes are handled:

- **A missing or `null` claim fails every rule**, including `notEquals`, `notIn`, and `exists`. A rule never passes because a claim is absent.
- **Single-value operators (`equals`, `notEquals`, `in`, `notIn`) never match an array claim**, not even a one-element array: `["web-client"]` does not satisfy `equals "web-client"`, and it does not satisfy `notIn` either. Use an array operator to test arrays.
- **Array operators (`contains`, `containsAny`, `containsAll`) accept a single value as a one-element array.** This covers claims that identity providers emit either way — [RFC 7519](https://datatracker.ietf.org/doc/html/rfc7519#section-4.1.3) allows `aud` to be a single string when there is only one audience — so `aud: "api"` and `aud: ["api", "web"]` both satisfy `contains "api"`. For claims that may arrive in either shape, prefer `contains` or `containsAny` over `equals` or `in`.
- **Array comparisons are set-based.** Element order and duplicates are ignored. Array elements that cannot match (objects, nested arrays, decimal numbers, `null`) are skipped. An empty array fails every array operator.
- **Object claims** fail every operator except `exists`.
- Exact array equality and "every claim element is in an allowed list" checks are not supported.

For example, a token carrying the claims below satisfies each of the rules that follow:

```json
{
  "appid": "web-client",
  "tenant_level": 3,
  "email_verified": true,
  "groups": ["admins", "editors"]
}
```

```yaml
openId:
  requiredJwtClaims:
    - claim: appid
      operator: equals
      value: "web-client"
    - claim: appid
      operator: in
      values: ["web-client", "cli-client"]
    - claim: tenant_level
      operator: notIn
      values: [0, 1]
    - claim: email_verified
      operator: equals
      value: true
    - claim: groups
      operator: containsAny
      values: ["admins", "auditors"]
    - claim: groups
      operator: containsAll
      values: ["admins", "editors"]
    - claim: email_verified
      operator: exists
```

The rules are validated when the service starts: an unknown operator, a missing or unexpected `value`/`values`, an empty `values` list, a decimal, array, or object used as a value, or an empty claim name makes the service exit with an error instead of starting without the check. Like the rest of the `openId` section, the rules reach the container as an environment variable, so changing them rolls the pods on the next upgrade.

The rules are evaluated against the claim set of the verified token: the JWT claims for `jwt`, the JWT claims before the `userinfo` response is merged in for `jwtuserinfo`, and the `userinfo` response for `opaque` (see {ref}`Token verification types <permission-service-authentication-token-verification-types>`).

(permission-service-authentication-what-happens-when-the-service-starts)=
### What happens when the service starts

When `openId.enabled` is `true`, the service performs the following steps before it begins accepting traffic:

1. It fetches the discovery document from `openId.openIdConfigurationUri`. Startup fails if the URL is unreachable or does not return a JSON document with the expected OIDC fields.
2. It extracts and stores the `jwks_uri`, `userinfo_endpoint`, `token_endpoint`, and `issuer` fields. Every subsequent token verification uses the endpoints from this cached response, so the chart only needs to know the discovery URL.
3. It initializes the in-memory **auth cache** for validated tokens (size `config.tokenCacheSize`, TTL `config.tokenTtl`), a separate **negative cache** for rejected tokens (size `config.invalidTokenCacheSize`, TTL `config.invalidTokenCacheTtl`), and the JWKS cache (TTL `config.jwksTtl`).

If the discovery document cannot be fetched or parsed, the service logs `Failed to initialize authentication` and exits immediately instead of starting without token validation. Kubernetes restarts the container, so an identity provider that is unreachable at deployment time shows up as a pod in `CrashLoopBackOff`, with the exit repeating until the provider answers. The service recovers on its own once the discovery URL responds; no redeployment is needed.

(permission-service-authentication-how-incoming-requests-are-authenticated)=
### How incoming requests are authenticated

Every authenticated REST or gRPC call goes through the same pipeline — see {ref}`Identity Verification <permission-service-policy-evaluation-1-identity-verification>` for how the verified principal is then used by the rest of the service. The authentication step itself is:

1. Extract the bearer token from the `Authorization` header. If it is missing or malformed, return `401 Unauthorized`.
2. Look the token up in the negative cache, then in the auth cache. A negative-cache hit returns `401 Unauthorized` immediately; an auth-cache hit reuses the cached principal. Neither contacts the IdP.
3. On a miss, validate the token according to `openId.tokenVerificationType` (see below), then check its claims against `openId.requiredJwtClaims` (see {ref}`Required claims <permission-service-authentication-required-claims>`); a token that fails any rule is answered with `401 Unauthorized`. Validated tokens are cached in the auth cache until the earlier of their `exp` claim and `config.tokenTtl`. Rejected tokens are recorded in the negative cache for `config.invalidTokenCacheTtl`, so repeated retries with the same bad token do not reach the IdP.
4. Build the requester principal from the validated claims: its id is the value of `openId.principalIdClaim` (default `sub`), and every claim is exposed as an attribute for Cedar conditions later in the pipeline.

The negative cache is separate from the auth cache, so a flood of invalid tokens cannot evict verified principals:

| Value                          | Default | Purpose |
|--------------------------------|---------|---------|
| `config.invalidTokenCacheSize` | `256`   | Maximum number of rejected tokens kept in the negative cache. |
| `config.invalidTokenCacheTtl`  | `60s`   | How long a rejected token stays in the negative cache. Capped at `config.tokenTtl` at startup. |

(permission-service-authentication-token-verification-types)=
### Token verification types

`openId.tokenVerificationType` selects one of three strategies. All three reuse the endpoints discovered from `openId.openIdConfigurationUri`.

(permission-service-authentication-jwt)=
#### `jwt`

The token is treated as a signed JWT. Verification is fully local after the JWKS fetch:

- The service reads the `kid` from the JWT header and looks up the matching key in the JWKS cache. On a cache miss or expiry (`config.jwksTtl`), it fetches `jwks_uri` from the discovery document and refreshes the cache. RSA and elliptic-curve keys are supported.
- It verifies the signature using the resolved key and checks the time window with `config.jwtLeeway` seconds of clock skew: the token must carry an `exp` claim that has not passed, and a token whose `nbf` (not-before) time is still in the future is rejected.
- It requires an `aud` claim that matches one of the audiences listed in `allowedJwtAudience`, and, when `allowedJwtIssuers` is set, an `iss` claim that matches one of the listed issuers. A token that omits a required claim is rejected the same way as one whose claim does not match.
- It extracts the principal id from the `principalIdClaim` claim (falling back to `sub`), checks the JWT claims against `requiredJwtClaims`, and exposes every JWT claim as a principal attribute. `exp` is kept so that the auth cache entry expires when the token itself does.

Use this mode when the identity provider issues JWT access tokens and all the claims you need in policies are already embedded in the token. It is the default and the cheapest — there is no per-request round-trip to the IdP after the JWKS has been cached.

(permission-service-authentication-opaque)=
#### `opaque`

The token is treated as an opaque string. Verification is delegated to the provider:

- The service sends `GET <userinfo_endpoint>` with `Authorization: Bearer <token>`. A `200 OK` response means the token is valid; the JSON body of that response is used as the claim set for the principal, and it is the claim set `requiredJwtClaims` is checked against.
- `401` is cached as a negative result; `403` is returned as `403 Forbidden` to the caller.
- No `aud` check is performed, so `allowedJwtAudience` is not required in this mode.
- Because opaque tokens do not carry an `exp`, the resulting principal is cached for `config.tokenTtl`. Set `config.tokenTtl` no higher than your provider's actual token lifetime, otherwise a revoked token will keep working until the cache entry expires.

Use this mode when the identity provider hands out opaque reference tokens (common for providers that enforce token introspection or want to keep claims server-side).

(permission-service-authentication-jwtuserinfo)=
#### `jwtuserinfo`

Runs the `jwt` verification first, then additionally calls `userinfo`:

- If the JWT passes every `jwt` check above and its claims satisfy `requiredJwtClaims`, the service fetches `userinfo` with the same bearer token. Claims that only `userinfo` returns cannot be used in `requiredJwtClaims` in this mode.
- The JSON returned by `userinfo` is merged on top of the JWT claims, so the principal exposes both sets of attributes for Cedar conditions. If the JWT does not carry `principalIdClaim` (nor `sub`), the id is taken from `userinfo` instead.

Use this mode when the JWT is enough to prove identity but some attributes you want to reference in policies (for example, group memberships, tenant id, or custom profile fields) live only on the `userinfo` endpoint. The cost is one extra HTTP round-trip per token — mitigated by `config.tokenCacheSize` and `config.tokenTtl`.

(permission-service-authentication-configuring-microsoft-entra-id)=
## Configuring Microsoft Entra ID

[Microsoft Entra ID](https://learn.microsoft.com/entra/identity/) (formerly Azure Active Directory) issues JWT access tokens signed with rotating RSA keys that are published through a standard OIDC discovery document. It is a straightforward fit for `tokenVerificationType: "jwt"`.

**Prerequisites on the Entra side.** In the Azure portal:

1. Register an **App registration** for the API (the Permission Service). Under *Expose an API*, add at least one scope (for example, `access_as_user`) so that client apps have something to request. Note the registration's **Application (client) ID** (`<api-client-id>`) — this is the GUID that Entra puts into the `aud` claim of v2.0 access tokens issued for this API.
2. Register a separate **App registration** for each client that will call Permission Service (web app, CLI, or service). Grant it at least one delegated or application permission on the API registration from step 1.
3. Note the directory's **tenant id** (`<tenant-id>`) and, for each client, its **client id** (`<web-client-id>`, `<cli-client-id>`, and so on).

The OIDC discovery URL for a tenant is:

```text
https://login.microsoftonline.com/<tenant-id>/v2.0/.well-known/openid-configuration
```

**Permission Service values.** A minimal Entra-backed configuration looks like this:

```yaml
openId:
  enabled: true
  tokenVerificationType: "jwt"
  openIdConfigurationUri: "https://login.microsoftonline.com/<tenant-id>/v2.0/.well-known/openid-configuration"

  allowedJwtAudience:
    - "<api-client-id>"

  principalIdClaim: "oid"
```

Notes specific to Entra:

- Entra v2.0 access tokens issued for a custom API set `aud` to the **client id (GUID) of the API app registration** — not to any of the calling client ids. Put that GUID (the `<api-client-id>` from step 1 of the prerequisites) in `allowedJwtAudience` so it passes audience validation.
- Clients request tokens for the API using the scope `<api-client-id>/.default` (the `api://` Application ID URI form is **not** used here — use the bare client-id GUID). Any additional standard scopes (`openid`, `profile`, `email`) can be added alongside it.
- Entra exposes the stable per-user directory identifier as the **`oid`** claim; `sub` is pairwise (different per application) and not suitable as a cross-service user id. Set `principalIdClaim: "oid"` so Cedar policies can reference a stable principal across applications. For service principals (client-credentials grants), Entra populates `oid` with the service principal's object id.
- Group membership claims in Entra are off by default. Enable them in the *Token configuration* blade of the API app registration to get a `groups` claim, then scope policies to a group with `principal in Group::"<id>"` or reference it through `principal.groups` (see {ref}`Groups and membership <permission-service-policy-language-groups-and-membership>`). If your IdP emits memberships under a different claim, set `openId.principalGroupsClaim` accordingly.
- To accept tokens only from specific client applications, require the client application id with `openId.requiredJwtClaims`. Entra v2.0 access tokens carry it in the `azp` claim, and v1.0 access tokens in the `appid` claim:

  ```yaml
  openId:
    requiredJwtClaims:
      - claim: azp
        operator: in
        values: ["<web-client-id>", "<cli-client-id>"]
  ```

With those values applied and the pod restarted, Permission Service fetches the Entra discovery document at startup, caches the JWKS, and begins validating `Bearer` tokens issued by that tenant.

(permission-service-authentication-outbound-service-identity-serviceidentity)=
## Outbound service identity (`serviceIdentity`)

Permission Service is a client of other Omniverse services. Most notably, {ref}`Service notifications <permission-service-notifications-service-notifications>` are delivered through gRPC to the Event Aggregation (Notification) Service, and publishing there is itself an authorized action (`Action::"event-aggregation-service:publish-event"`). Anything Permission Service calls over the network needs its own bearer token; the **`serviceIdentity`** section of the chart configures how that token is obtained.

`serviceIdentity` offers two mutually exclusive modes:

- **Client-credentials flow** — Permission Service exchanges a client id and a client secret for an access token at the provider's token endpoint, and refreshes the token as it expires.
- **Access-token file** — A sidecar container writes a pre-provisioned access token into a shared in-memory volume, and Permission Service reads it from there. No secret material lives inside the Permission Service container.

Exactly one of `serviceIdentity.clientCredentials.enabled` and `serviceIdentity.accessTokenFile.enabled` may be `true`. When both are `false`, outgoing calls are made without a bearer token — acceptable only for downstream services that do not require authentication (typically only in tests).

(permission-service-authentication-client-credentials-flow)=
### Client credentials flow

```yaml
serviceIdentity:
  clientCredentials:
    enabled: true
    openIdConfigurationUri: "https://idp.example.com/.well-known/openid-configuration"
    clientId: "permission-service"
    clientScope: "notifications.publish"
    clientSecretRef:
      name: permission-service-client-credentials
      key: clientSecret
    extraHeaders: {}
    extraBodyParams: {}
```

| Value                                                     | Purpose |
|-----------------------------------------------------------|---------|
| `serviceIdentity.clientCredentials.enabled`               | Turns the client-credentials flow on. Default `false`. |
| `serviceIdentity.clientCredentials.openIdConfigurationUri`| Discovery URL of the IdP used for the token exchange. When empty, the chart falls back to `openId.openIdConfigurationUri` — handy when the same IdP issues both inbound and outbound tokens. |
| `serviceIdentity.clientCredentials.clientId`              | Client id registered at the IdP for Permission Service. |
| `serviceIdentity.clientCredentials.clientScope`           | Value of the `scope` form field sent with the token exchange. |
| `serviceIdentity.clientCredentials.clientSecretRef`       | Reference to a pre-existing Kubernetes `Secret` containing the client secret. The chart mounts it at `/etc/secrets/client-credentials/<key>` and passes the path to the service; the secret is never written into a ConfigMap or a values snapshot. |
| `serviceIdentity.clientCredentials.extraHeaders`          | Optional JSON object of additional HTTP headers to attach to the token exchange request (for example `{"X-Tenant": "omniverse"}`). |
| `serviceIdentity.clientCredentials.extraBodyParams`       | Optional JSON object of additional body form parameters (for example `{"audience": "https://notifications.example.com"}` when the IdP requires a resource audience). |

At startup, the service fetches the discovery document from `openIdConfigurationUri` (or falls back to `openId.openIdConfigurationUri`) and extracts the `token_endpoint`. Every outgoing call then carries the currently cached access token in its `Authorization` header; when the cached token has expired or the downstream service answers `Unauthenticated`, the service exchanges a fresh token against the token endpoint — posting `grant_type=client_credentials` together with the client id, the secret read from the mounted file, the configured scope, and any `extraHeaders`/`extraBodyParams` — and caches the new token for its reported `expires_in`.

Use this mode when your IdP supports the standard OAuth 2.0 client-credentials grant and you are happy storing a long-lived client secret in a Kubernetes `Secret`.

(permission-service-authentication-access-token-file)=
### Access-token file

```yaml
serviceIdentity:
  accessTokenFile:
    enabled: true
    pollInterval: "15s"
    provisionerSidecar:
      container:
        name: token-provisioner
        image: registry.example.com/token-provisioner:1.0.0
        args: ["--output", "/etc/secrets/access-token/token"]
```

| Value                                                            | Purpose |
|------------------------------------------------------------------|---------|
| `serviceIdentity.accessTokenFile.enabled`                        | Turns the access-token-file mode on. Default `false`. |
| `serviceIdentity.accessTokenFile.pollInterval`                   | How often the main container re-reads the shared token file (humantime format, for example `15s`, `1m`). Default `15s`. |
| `serviceIdentity.accessTokenFile.provisionerSidecar.container`   | Full container spec for a sidecar that writes the access token to the shared volume. The chart takes this spec verbatim and appends a `token-exchange` volume mount pointing at `/etc/secrets/access-token`, so the sidecar only needs to write `token` into that directory. |

This mode lets you **deploy the Permission Service alongside any token-provisioning container you already use** — a workload-identity helper, a Vault agent, a Kubernetes service-account token projector, or a cloud-specific metadata client. The sidecar runs in the same Pod and shares an in-memory (`emptyDir` with `medium: Memory`) volume called `token-exchange`, mounted at `/etc/secrets/access-token/` in both containers. The sidecar writes (and, over time, rewrites) a single file named `token` containing JSON with `access_token` and `expires_on` fields. `expires_on` must be an RFC3339 timestamp. The access token must be accepted by the downstream services Permission Service talks to — for example, a token with `event-aggregation-service:publish-event` granted when notifications are enabled.

Permission Service reads `/etc/secrets/access-token/token` at startup, watches it at `pollInterval`, and re-reads it eagerly whenever a downstream service returns `Unauthenticated`. The provisioner sidecar is fully under your control — from its image to its args to its volume mounts — which makes this the preferred mode in environments where:

- Short-lived tokens are minted from a platform-specific identity (workload identity federation, IRSA, AKS workload identity, GKE workload identity, and similar) and the Permission Service itself should not hold a client secret.
- The token exchange requires a flow (JWT assertion, mTLS, workload-attested OIDC) that is more complex than the standard OAuth 2.0 client-credentials grant supported by the built-in client.
- Several services in the same deployment need to share a single sidecar pattern.

See {ref}`Service notifications → Authenticating to the Notification Service <permission-service-notifications-authenticating-to-the-notification-service>` for the exact policy the presented principal needs on the Notification Service side, and for how Permission Service falls back to an anonymous channel when neither mode is enabled.

(permission-service-authentication-references)=
## References

- [OAuth 2.0 Authorization Framework (RFC 6749)](https://datatracker.ietf.org/doc/html/rfc6749) — the client-credentials grant (Section 4.4) that `serviceIdentity.clientCredentials` uses to obtain outbound tokens.
- [OpenID Connect Core 1.0](https://openid.net/specs/openid-connect-core-1_0.html) — the token and claim semantics (`sub`, `aud`, `exp`) validated on incoming bearer tokens.
- [OpenID Connect Discovery 1.0](https://openid.net/specs/openid-connect-discovery-1_0.html) — the `/.well-known/openid-configuration` document the service reads to locate the `jwks_uri`, `userinfo_endpoint`, and `token_endpoint`.
- [JSON Web Token (RFC 7519)](https://datatracker.ietf.org/doc/html/rfc7519) — section 4.1.3 allows the `aud` claim to be a single string or an array, which is why array operators in `requiredJwtClaims` accept a single value; section 4.1.5 defines the `nbf` (not-before) claim.
- [Microsoft Entra ID documentation](https://learn.microsoft.com/entra/identity/) — the identity provider used in the Entra configuration example above.
- [Microsoft identity platform access token claims reference](https://learn.microsoft.com/entra/identity-platform/access-token-claims-reference) — the `azp` (v2.0) and `appid` (v1.0) client application id claims used in the Entra `requiredJwtClaims` example.

(permission-service-authentication-see-next)=
## See next

- {ref}`Writing Cedar policies for Permission Service <permission-service-policy-language-writing-cedar-policies-for-permission-service>` — the policy language that decides what an authenticated principal may do, and how the verified principal and its claims become `principal` in Cedar.

Related pages:

- {ref}`Service notifications <permission-service-notifications-service-notifications>` — the primary consumer of the `serviceIdentity` credentials.
- {ref}`How authorization requests are evaluated <permission-service-policy-evaluation-how-authorization-requests-are-evaluated>` — pipeline that runs after a token has been validated.
- {ref}`Permission Service Configuration <permission-service-deployment-permission-service-configuration>` — full Helm chart reference, including `openId` in its wider context.
- {ref}`Managing service metadata through the REST API <permission-service-metadata-managing-service-metadata-through-the-rest-api>` — per-service `idClaim` override that complements `openId.principalIdClaim`.
