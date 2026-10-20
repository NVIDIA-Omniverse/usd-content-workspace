(userinfo-service-identity-provider-microsoft-entra-id-integration)=
# Microsoft Entra ID Integration

User Info Service reads all of its directory data from Microsoft Entra ID, the organization's
identity provider, through the [Microsoft Graph API][graph]. This page describes how the service
authenticates to the provider as an application, the directory permissions it requires, the two
credential options it supports, how it is configured in the Helm chart, every Graph call and Entra
endpoint it uses, and how it stays within the provider's request limits.

The service never acts on behalf of a signed-in user when reading the directory. It authenticates
as itself using its own identity — either a Microsoft Entra application registration or a
user-assigned managed identity — so every call to Microsoft Graph carries the service's own token
and is evaluated against that identity's permissions, regardless of which caller triggered the
request.

(userinfo-service-identity-provider-application-identity)=
## Application Identity and App-Only Authentication

The service reads the directory with an **application identity of its own**. It never signs in a
user and never uses a caller's token to read Graph — every Graph call carries the service's own
access token and is evaluated against the permissions granted to that identity.

Two kinds of Entra identity can back the service, distinguished by how the credential is supplied:

- A **Microsoft Entra application registration** (a client), authenticated either with a client
  secret or with a federated credential.
- A **user-assigned managed identity**, authenticated through Microsoft Entra Workload ID
  federation. In this case there is no application registration to create and no secret to store;
  the tenant and client identifiers point at the managed identity instead.

Whichever identity is used, the service obtains an app-only access token for the scope
`https://graph.microsoft.com/.default` from the tenant's token endpoint:

- `POST https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token`

The `.default` scope grants the identity the application permissions that have been consented for
it in the tenant. Because no signed-in user is involved, the identity must be granted
**application permissions**, not delegated ones.

How the identity proves itself at the token endpoint depends on the configured authentication mode:

- **Client secret** — the application registration presents its client identifier and a client
  secret. This is the OAuth 2.0 [client-credentials flow][client-creds] and requires an
  application registration; a managed identity cannot be used this way.
- **Workload identity federation** — the application registration or user-assigned managed identity
  presents a short-lived, platform-issued token as a signed client assertion instead of a secret.
  On Kubernetes this assertion token is projected into the container as a file and refreshed
  automatically; the service reads it from that file on each token request. See
  [Workload identity federation][workload-id] for how the trust between the Kubernetes service
  account and the Entra identity is established.

The returned access token is held in memory and reused for subsequent Graph calls until it
approaches expiry, at which point the service requests a fresh one. Tokens are refreshed slightly
ahead of their stated expiry so that in-flight requests are not sent with a token that is about to
expire, and only one token request is made at a time even when many directory requests arrive
together.

(userinfo-service-identity-provider-permissions)=
## Required Directory Permissions and Administrator Consent

Because the service uses app-only authentication, it needs Microsoft Graph **application
permissions** granted with tenant administrator consent. Delegated permissions do not apply, as no
user participates in the flow. Each permission below must be consented in the tenant before the
service can read the corresponding data; see the
[Microsoft Graph permissions reference][permissions] for details.

| Data | Permission | Why it is needed |
|---|---|---|
| Users | `User.Read.All` | Read user profiles for the get and list operations and for user change tracking. |
| Groups | `Group.Read.All` | Read group properties for the get and list operations and for group change tracking. |
| Group membership | `GroupMember.Read.All` | Read group members, a user's group memberships, membership checks, and group change tracking when membership data is included. Required only when group-membership queries or synchronization are enabled. |

One additional permission applies only in a specific scoped configuration. When the served
directory is narrowed to a single enterprise application and that scope is pointed at a **different**
application's service principal, the service also needs `Application.Read.All`. When the scope is
left to the service's own application — the default — no additional permission is required, because
an application may read its own service principal's assignments. See {ref}`Directory Scoping
<userinfo-service-directory-scoping-directory-scoping>` for the full behavior of the scoped configuration.

(userinfo-service-identity-provider-credential-options)=
## Credential Options

The two authentication modes are mutually exclusive and produce the same app-only token; they
differ only in how the credential is supplied and stored:

- **`client-secret`** — an application registration authenticated with a client secret. The secret
  is supplied as a Kubernetes Secret and read at runtime; it is not written into the Helm values.
- **`workload-identity`** — an application registration or a user-assigned managed identity
  authenticated with a federated, platform-issued assertion token. No long-lived secret is stored,
  which makes this the recommended option where the cluster and tenant are configured for it.

The settings below configure the identity and its credential.

| Setting | Environment variable | Helm value (`providers.entra.*`) | Default | Description |
|---|---|---|---|---|
| Tenant identifier | `AZURE_TENANT_ID` | `tenantId` | _(required)_ | The Entra tenant the identity belongs to. |
| Client identifier | `AZURE_CLIENT_ID` | `clientId` | _(required)_ | The client identifier of the application registration, or of the user-assigned managed identity when using workload identity federation. |
| Authentication mode | `AZURE_AUTH_MODE` | `authMode` | `client-secret` | Credential option to use: `client-secret` or `workload-identity`. |
| Client secret | `AZURE_CLIENT_SECRET` | `clientSecret.secretRef` | _(unset)_ | The client secret, supplied from a Kubernetes Secret. Used only with `client-secret`. |
| Federated token file | `AZURE_FEDERATED_TOKEN_FILE` | _(none; not rendered by the chart)_ | `/var/run/secrets/azure/tokens/azure-identity-token` | Path to the projected assertion token presented as a client assertion. Used only with `workload-identity`. The Microsoft Entra Workload ID webhook sets this variable on labeled pods; when it is not set, the service uses the default path. |

For step-by-step credential setup — creating the client-secret Secret or annotating the service
account for workload identity federation — see {ref}`User Info Service Configuration <userinfo-service-deployment-configuration>`.

(userinfo-service-identity-provider-helm-configuration)=
## Configuring the Identity Provider in the Helm Chart

The identity provider is configured under the `providers.entra` section of the chart's values,
together with the pod's service account. The chart translates these values into the settings listed
in {ref}`Credential Options <userinfo-service-identity-provider-credential-options>` and wires up
whatever the chosen authentication mode requires.

| Value (`providers.entra.*`) | Purpose |
|---|---|
| `tenantId` | The Entra tenant identifier. |
| `clientId` | The client identifier of the application registration or user-assigned managed identity. |
| `authMode` | The authentication mode: `client-secret` or `workload-identity`. |
| `clientSecret.secretRef.{name,key}` | The Kubernetes Secret and key holding the client secret. Used only with `client-secret`. |
| `maxRetries` | Maximum retries for transient Graph errors. |

(userinfo-service-identity-provider-helm-client-secret)=
### Client-Secret Mode

Reference a pre-created Kubernetes Secret that holds the client secret; the secret itself is never
placed in the values file:

```yaml
providers:
  entra:
    tenantId: "{AZURE_TENANT_ID}"
    clientId: "{AZURE_CLIENT_ID}"
    authMode: "client-secret"
    clientSecret:
      secretRef:
        name: "microsoft-graph-userinfo-service-az-credentials"
        key: "client-secret"
    maxRetries: 3
```

The chart reads the secret from that Secret at runtime and supplies it to the service.

(userinfo-service-identity-provider-helm-workload-identity)=
### Workload-Identity Mode

Set the authentication mode to `workload-identity` and omit the client-secret Secret. Here `clientId`
is the client identifier of the application registration or user-assigned managed identity that the
Kubernetes service account is federated with:

```yaml
providers:
  entra:
    tenantId: "{AZURE_TENANT_ID}"
    clientId: "{AZURE_CLIENT_ID}"
    authMode: "workload-identity"
    maxRetries: 3

serviceAccount:
  create: true
```

In this mode the chart automatically:

- labels the writer and reader pods with `azure.workload.identity/use: "true"`, which tells the
  Microsoft Entra Workload ID webhook to project a short-lived assertion token into the container;
- annotates the chart-created service account with `azure.workload.identity/client-id` set to
  `clientId`, binding the pods to that identity;
- leaves the client secret unset.

The chart does not set the federated token file itself. On each labeled pod, the webhook mounts
the projected token and sets the `AZURE_FEDERATED_TOKEN_FILE` environment variable to its path,
which the service reads; when the variable is not set, the service falls back to its default path,
`/var/run/secrets/azure/tokens/azure-identity-token`. There is therefore no Helm value for the
token file. If you provide your own service account instead of
letting the chart create one (`serviceAccount.create: false` with `serviceAccount.name`), annotate
that account with `azure.workload.identity/client-id` yourself — the chart only annotates the
account it creates.

Establishing the federated trust in Entra — creating the federated credential that links the
Kubernetes service account to the application registration or managed identity — is a separate,
cluster- and tenant-specific step; see {ref}`User Info Service Configuration <userinfo-service-deployment-configuration>`.

(userinfo-service-identity-provider-calling-graph)=
## How the Service Calls Microsoft Graph

All directory reads go to the Microsoft Graph version 1.0 endpoint,
`https://graph.microsoft.com/v1.0`. Each request carries the application's access token and a
consistent set of conventions:

- **`ConsistencyLevel: eventual` header** — sent on every request, as it is required for the
  `$search` and `$count` query options the service uses.
- **`$select` query option** — every request names only the properties the service needs, so
  responses stay small. For users these are the identifier, display name, given and family name,
  email, username, account-enabled status, and job title; for groups, the identifier, display
  name, and description.
- **`client-request-id` header** — every request includes a unique identifier, as recommended by
  the [Microsoft Graph best practices][best-practices]. Microsoft records this value in its Graph
  activity logs, which makes it useful when raising a support request. When tracing is enabled this
  identifier is derived from the active trace so the service's own traces line up with Microsoft's
  server-side records.

List and search requests additionally use the OData query options `$top` (page size), `$count`,
`$filter` (exact matches such as email address), and `$search` (name search). Results are paged
through the opaque `@odata.nextLink` URLs returned by Graph. See the
[OData query parameters][query-parameters] reference for these options.

(userinfo-service-identity-provider-graph-calls)=
## Graph Calls and Entra Endpoints Used

The table below enumerates every integration the service makes with Microsoft Entra ID and
Microsoft Graph, including the one direct call to the Entra token endpoint and each Graph
collection or resource it reads.

| Integration | Endpoint | When it is used |
|---|---|---|
| Acquire application token | `POST https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token` | Before any Graph call, to obtain and refresh the app-only access token. |
| List users | `GET /users` | Listing and searching users when user data is served live from the provider. |
| Get a user | `GET /users/{id}` | Fetching a single user, and resolving the current caller (see below). |
| List groups | `GET /groups` | Listing and searching groups when group data is served live. |
| Get a group | `GET /groups/{id}` | Fetching a single group, and fetching each assigned group in the scoped configuration. |
| List group members | `GET /groups/{id}/members/microsoft.graph.user` | Listing a group's direct user members, membership checks, and enumerating members of assigned groups in the scoped configuration. |
| List a user's groups | `GET /users/{id}/memberOf/microsoft.graph.group` | Listing the groups a user belongs to and checking a specific membership. |
| Track user changes | `GET /users/delta` | Synchronizing users — the initial full read and each incremental cycle. |
| Track group changes | `GET /groups/delta` | Synchronizing groups and, when enabled, group memberships via the `members` selection. |
| List application assignments | `GET /servicePrincipals/{id}/appRoleAssignedTo` | Reading an enterprise application's assigned principals, only in the scoped configuration. |
| Resolve own service principal | `GET /servicePrincipals(appId='{clientId}')` | Finding the service's own service principal in the scoped configuration when no object identifier is set. |

The change-tracking (`delta`) calls and the scoped-configuration calls are described in more
detail in {ref}`Directory Synchronization and Data Freshness <userinfo-service-directory-synchronization-directory-synchronization-and-data-freshness>` and
{ref}`Directory Scoping <userinfo-service-directory-scoping-directory-scoping>`.

(userinfo-service-identity-provider-current-caller)=
## Resolving the Current Caller

The current-user request (`GET /users/me` and its gRPC equivalent) identifies the caller from the
access token they present. Microsoft Entra access tokens carry an object identifier (`oid`) that
uniquely names the caller in the tenant. The service reads this identifier from the caller's token
and looks the caller up as an ordinary directory user — from the local replica when user data is
synchronized, or with a `GET /users/{id}` call otherwise.

Validating the caller's token — checking its signature, issuer, audience, and expiry — is the
responsibility of the platform's authentication gateway in front of the service. The service reads
the object identifier from an already-validated token only to resolve the caller's directory
record; it is not used as an authorization decision. Authorization, when enabled, is handled
separately; see {ref}`Authorization with the Permission Service <userinfo-service-authorization-authorization-with-the-permission-service>`.

(userinfo-service-identity-provider-rate-limits)=
## Rate Limits and Transient Errors

Microsoft Graph enforces per-tenant request limits and may occasionally return temporary errors.
Because every instance of the service issues its own Graph calls, the combined request rate grows
with the number of instances, so the service is deliberately conservative and cooperative with
other consumers of the same tenant. See the [Graph throttling guidance][throttling] for the
provider's side of this behavior.

- **Rate-limit responses (`429`).** When Graph reports that the request rate is too high, the
  service waits for the amount of time the provider specifies in its `Retry-After` response before
  trying again, rather than retrying immediately.
- **Transient server errors (`503`, `504`).** When Graph returns a temporary server error, the
  service backs off and retries with an increasing delay between attempts, spread with a small
  random offset so that multiple instances do not retry in lockstep.
- **Retry limit.** Each request is retried up to a configurable number of times before the failure
  is surfaced. Errors that are not transient — such as a missing user — are not retried.

| Setting | Environment variable | Helm value (`providers.entra.*`) | Default | Description |
|---|---|---|---|---|
| Maximum retries | `GRAPH_MAX_RETRIES` | `maxRetries` | `3` | How many times a rate-limited or transient Graph error (`429`, `503`, `504`) is retried before the request fails. |

Change-tracking state can also become invalid on the provider's side — for example when a saved
change-tracking link expires. In that case Graph responds with a `410 Gone` status or an error
indicating that a resync is required, and the writer discards the saved state and performs a fresh
full synchronization on its next cycle. This recovery behavior is described in
{ref}`Directory Synchronization and Data Freshness <userinfo-service-directory-synchronization-directory-synchronization-and-data-freshness>`.

(userinfo-service-identity-provider-references)=
## References

- [Microsoft Graph REST API][graph] — the interface the service reads directory data through.
- [Register an application with the Microsoft identity platform][register-app] — creating the
  application registration the service authenticates as.
- [OAuth 2.0 client-credentials flow][client-creds] — the app-only authentication flow used to
  obtain a Graph token.
- [Workload identity federation][workload-id] — secretless credential option using a projected
  service-account token.
- [Microsoft Entra Workload ID on AKS][aks-workload-id] — the `azure.workload.identity/use` pod
  label, and the webhook that projects the service-account token and sets
  `AZURE_FEDERATED_TOKEN_FILE` to its path.
- [Microsoft Graph permissions reference][permissions] — the application permissions and
  administrator consent the service requires.
- [OData query parameters][query-parameters] — `$select`, `$filter`, `$search`, `$top`, `$count`,
  and the `ConsistencyLevel: eventual` header.
- [Microsoft Graph best practices][best-practices] — the `client-request-id` header and related
  request conventions.
- [Microsoft Graph throttling guidance][throttling] — rate limits, `Retry-After` headers, and
  backoff.

[graph]: https://learn.microsoft.com/en-us/graph/use-the-api
[register-app]: https://learn.microsoft.com/en-us/entra/identity-platform/quickstart-register-app
[client-creds]: https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-client-creds-grant-flow
[workload-id]: https://learn.microsoft.com/en-us/entra/workload-id/workload-identity-federation
[aks-workload-id]: https://learn.microsoft.com/en-us/azure/aks/workload-identity-overview
[permissions]: https://learn.microsoft.com/en-us/graph/permissions-reference
[query-parameters]: https://learn.microsoft.com/en-us/graph/query-parameters
[best-practices]: https://learn.microsoft.com/en-us/graph/best-practices-concept
[throttling]: https://learn.microsoft.com/en-us/graph/throttling

(userinfo-service-identity-provider-see-next)=
## See next

- {ref}`Authorization with the Permission Service <userinfo-service-authorization-authorization-with-the-permission-service>`
