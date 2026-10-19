(thumbnail-service-authentication-authenticating-with-an-openid-connect-identity-provider)=
# Authenticating with an OpenID Connect Identity Provider

Thumbnail Service does not mint or validate credentials of its own. Every authenticated call *into* the service
carries a bearer token that the service forwards to the Storage service, and every call the service and the
task-worker make *out* to other Omniverse services is backed by a bearer token issued by a separate **OpenID
Connect (OIDC)** identity provider. This page documents both sides of that integration as they are exposed
through the Helm chart:

- **Inbound**: what the REST and gRPC endpoints require of a caller's `Authorization` header, and who makes the
  authorization decision.
- **Outbound**: the top-level **`serviceIdentity`** section, which configures how both components obtain the
  bearer token for their own calls to the Storage service, the Notification Service, and — for the worker — the
  orchestrator.

For the variables this page maps to, see
{ref}`Outbound authentication <thumbnail-service-configuration-outbound-authentication>`. For which token each
storage call carries, see {ref}`Which credential is used <thumbnail-service-associations-which-credential-is-used>`.

(thumbnail-service-authentication-inbound-request-authentication)=
## Inbound request authentication

Every REST endpoint under `/v1alpha` and every `ThumbnailService` RPC requires `Authorization: Bearer <token>`
(as an HTTP header or as gRPC request metadata; the scheme is matched case-insensitively). The service checks
**presence only** — it never parses the token, validates its signature, or checks its expiry. The token is
forwarded unchanged on the storage calls the request makes, and the Storage service's permission check is the
authorization decision.

The check runs **before** the request body is parsed, so an unauthenticated caller sending a malformed payload
is told about the missing token, not handed the schema it got wrong.

| Situation | HTTP | gRPC |
|---|---|---|
| No `Authorization` header, or not `Bearer <token>` | `401 Unauthorized` | `UNAUTHENTICATED` |
| Storage rejects the token (expired, revoked, wrong audience) | `401 Unauthorized` | `UNAUTHENTICATED` |
| Storage accepts the token and denies access to the resource (`CreateThumbnails` source, `GetThumbnails` target) | `403 Forbidden` | `PERMISSION_DENIED` |

A `401` invites a new token; a `403` does not — a new token will not help. The two are deliberately kept apart
on the create and read paths.

Three surfaces are unauthenticated by design: `GET /health` and `GET /health/live` (Kubernetes probes call them
without a token), and the gRPC `CapabilitiesService/ListServices` RPC, which the discovery operator dials over a
plaintext, credential-less channel.

```{warning}
`DEVELOPMENT_MODE=true` makes `GET /v1alpha/thumbnails` return fabricated data without calling storage. Because
storage's permission check is the **only** authorization the read path has, this bypasses authorization
entirely: any bearer-shaped token gets a `200`. It exists for local development, has no Helm value, and must
never be set in a deployed environment.
```

(thumbnail-service-authentication-outbound-service-identity-serviceidentity)=
## Outbound service identity (`serviceIdentity`)

Both Deployments are clients of other Omniverse services. The orchestrator publishes to the Notification
Service, provisions and consumes its durable queue, and — during storage-deleted cleanup — deletes blobs under
its own identity; the worker reads and writes storage blobs, consumes its queue, and calls `AssociateThumbnail`.
Every one of those calls carries a bearer token obtained through the **`serviceIdentity`** section of the chart,
which both Deployments share so they authenticate as **one external principal**.

`serviceIdentity` offers two mutually exclusive modes:

- **Access-token file** (recommended) — a sidecar container writes a pre-provisioned access token into a
  shared in-memory volume, and the process reads it from there. No secret material lives inside the service
  containers.
- **Client-credentials flow** — the process exchanges a client id and a client secret for an access token at
  the provider's token endpoint and refreshes the token itself. Reserved for local development and testing;
  see {ref}`Client credentials flow <thumbnail-service-authentication-client-credentials-flow>`.

Exactly one of `serviceIdentity.clientCredentials.enabled` and `serviceIdentity.accessTokenFile.enabled` must be
`true`. The chart enforces this at render time and both binaries fail fast at startup if both or neither mode is
enabled. Unlike some sibling services there is no anonymous fallback: with neither mode configured the process
does not start.

(thumbnail-service-authentication-client-credentials-flow)=
### Client credentials flow

```{code-block} yaml
:caption: thumbnail-values.yaml

serviceIdentity:
  clientCredentials:
    enabled: true
    clientId: "<application-client-id>"
    clientScope: "<application-client-id>/.default"
    openIdConfigurationUri: "https://login.microsoftonline.com/<tenant-id>/v2.0/.well-known/openid-configuration"
    clientSecretRef:
      name: thumbnail-service-client-credentials
      key: CLIENT_SECRET
    extraHeaders: {}
    extraBodyParams: {}
```

| Value | Purpose |
|---|---|
| `serviceIdentity.clientCredentials.enabled` | Turns the client-credentials flow on (`ENABLE_CLIENT_CREDENTIALS=true`). Default `false`. |
| `serviceIdentity.clientCredentials.clientId` | Client (application) id registered at the identity provider (`CLIENT_ID`). Required. |
| `serviceIdentity.clientCredentials.clientScope` | Value of the `scope` form field sent with the token exchange (`CLIENT_SCOPE`). For Microsoft Entra ID this is `<application-client-id>/.default`. |
| `serviceIdentity.clientCredentials.openIdConfigurationUri` | URL of the provider's OIDC discovery document (`CLIENT_CREDENTIALS_OPENID_CONFIGURATION`); the `token_endpoint` is read from it at startup. Required. |
| `serviceIdentity.clientCredentials.clientSecretRef.name` | An **existing** Kubernetes `Secret` holding the client secret. Required; the chart never creates Secrets. |
| `serviceIdentity.clientCredentials.clientSecretRef.key` | Key inside that Secret (default `CLIENT_SECRET`; letters, digits, `.`, `_`, `-` only). Mounted read-only at `/etc/secrets/<key>` and passed as `CLIENT_SECRET_FILE`; the secret is never written into a ConfigMap or a rendered manifest. |
| `serviceIdentity.clientCredentials.extraHeaders` | Optional object of additional HTTP headers on the token request (`CLIENT_CREDENTIALS_EXTRA_HEADERS`). |
| `serviceIdentity.clientCredentials.extraBodyParams` | Optional object of additional form parameters on the token request (`CLIENT_CREDENTIALS_EXTRA_BODY_PARAMS`), for example a `resource` or `audience` an identity provider requires. |

(thumbnail-service-authentication-what-happens-when-the-service-starts)=
#### What happens when the service starts

1. The process fetches the discovery document from `openIdConfigurationUri` and stores its `issuer` and
   `token_endpoint`. Startup fails if the URL is unreachable or the document lacks those fields.
2. It starts a background token manager that posts `grant_type=client_credentials` with the client id, the
   secret read from the mounted file, the configured scope, and any extra headers or body parameters to the
   token endpoint, and caches the returned token for its `expires_in`.
3. It waits for a usable token — see {ref}`Token readiness at startup <thumbnail-service-authentication-token-readiness-at-startup>` — and only then connects its outbound channels.

At runtime a gRPC interceptor stamps the cached token onto every outbound call. When the cached token nears its
expiry the manager refreshes it in the background; when a downstream service answers `UNAUTHENTICATED`, the call
site refreshes once and reissues the same request once. A second `UNAUTHENTICATED` is treated as permanent.

Reserve this mode for local development and testing. It stores a long-lived client secret in a Kubernetes
`Secret`; prefer {ref}`access-token file <thumbnail-service-authentication-access-token-file>` for every other
deployment, since it keeps no secret material in the chart's values or in the service containers.

(thumbnail-service-authentication-access-token-file)=
### Access-token file

```{code-block} yaml
:caption: thumbnail-values.yaml

serviceIdentity:
  clientCredentials:
    enabled: false
  accessTokenFile:
    enabled: true
    path: "/etc/secrets/access-token/token"
    pollInterval: "15s"
    provisionerSidecar:
      container:
        image: "<registry>/client-credentials-sidecar:<tag>"
        env:
          - name: TOKEN_OUTPUT_PATH
            value: "/etc/secrets/access-token/token"
        resources:
          requests:
            cpu: 10m
            memory: 32Mi
```

| Value | Purpose |
|---|---|
| `serviceIdentity.accessTokenFile.enabled` | Turns the access-token-file mode on (`ENABLE_ACCESS_TOKEN_FILE=true`). Default `false`. |
| `serviceIdentity.accessTokenFile.path` | Path the main containers read (`ACCESS_TOKEN_FILE_PATH`). Default `/etc/secrets/access-token/token`; must match where the sidecar writes. |
| `serviceIdentity.accessTokenFile.pollInterval` | How often the main containers re-read the file looking for a rotation (`ACCESS_TOKEN_FILE_POLL_INTERVAL`, humantime). Default `15s`. |
| `serviceIdentity.accessTokenFile.provisionerSidecar.container` | Full container spec for the sidecar that writes the token. Required non-empty when this mode is enabled. The chart renders it verbatim, names it `<chart>-token-provisioner` (`<chart>-worker-token-provisioner` on the worker), and appends a mount of the shared volume at `/etc/secrets/access-token`. |

The sidecar and the application container share an in-memory `emptyDir` volume (`medium: Memory`, 4 MiB) —
`token-exchange` on the orchestrator pod, `worker-token-exchange` on the worker pod — mounted at
`/etc/secrets/access-token/` in both. The sidecar writes, and over time rewrites, a single JSON file named
`token`:

```json
{
  "access_token": "eyJ0eXAiOiJKV1QiLCJhbGciOiJSUzI1NiIs...",
  "expires_on": "2026-09-11T18:04:30Z"
}
```

`expires_on` must be an RFC 3339 timestamp. The application reads the file at startup, re-reads it every
`pollInterval`, and re-reads it eagerly whenever a downstream service answers `UNAUTHENTICATED`. The token must
be accepted by every service the component talks to — storage, both Notification APIs, and (for the worker) the
orchestrator.

This mode lets you deploy the service alongside any token-provisioning container you already use — a workload
identity helper, a Vault agent, or a cloud metadata client. On Azure Workload Identity clusters, set the
federated-credential annotations under root `serviceAccount.annotations`, the required pod label under root
`podLabels`, and give both components one ServiceAccount name through root `serviceAccount.name` (the binding
is keyed to a single name).

Use this mode when short-lived tokens are minted from a platform identity and the containers should hold no
client secret, or when the exchange needs a flow richer than the standard client-credentials grant.

```{note}
The sidecar needs no resources of its own, and the chart ships none for it. Both HorizontalPodAutoscalers use a
`ContainerResource` metric targeting the main container by name (`thumbnail-service` / `thumbnail-worker`), so a
sidecar carrying no request does not dilute or block the scaling signal the way it did under the pod-aggregate
`Resource` metric. Set requests and limits from your environment values file if your cluster requires every
container to carry them.
```

(thumbnail-service-authentication-token-readiness-at-startup)=
### Token readiness at startup

Neither component opens an outbound channel with an empty bearer and "recovers" through `UNAUTHENTICATED`.
At startup each waits up to **60 seconds** (`TOKEN_READY_TIMEOUT`, a constant) for a usable token — a completed
client-credentials exchange, or a token file that exists and is outside its expiry skew — and fails the process
if none arrives. A slow identity provider or a late sidecar therefore becomes `CrashLoopBackOff` with an
explicit error instead of a running pod that serves `401`s.

The chart accounts for this wait: both Deployments carry a startup probe with a ~125 s budget so liveness does
not restart a pod that is still waiting for its first token, and the worker binds its probe listener before it
builds its outbound clients so `/healthz` answers immediately.

(thumbnail-service-authentication-which-token-is-used-for-which-call)=
### Which token is used for which call

| Call | Token |
|---|---|
| Storage `Stat` resolving the source for `CreateThumbnails` | the **caller's** bearer |
| Storage metadata reads and writes on `POST`/`DELETE /v1alpha/associations` and `GET /v1alpha/thumbnails` (and their RPCs) | the **caller's** bearer |
| Storage metadata enumeration and removal during `omni.storage.deleted` cleanup | the **service identity** |
| Storage blob deletion during that cleanup | the **service identity** — the worker wrote the blobs under it, and the original caller may lack delete rights on the thumbnail path |
| Publishing `omni.thumbnail.generate`, `omni.thumbnail.associated`, `omni.thumbnail.disassociated` | the **service identity** |
| Provisioning and consuming StorageEventQueue and WorkerEventQueue | the **service identity** |
| Worker storage reads and writes, and its `AssociateThumbnail` call to the orchestrator | the **service identity** |

Because the worker calls `AssociateThumbnail` with the service identity, the association metadata it triggers is
written to storage under that principal — the service account must therefore hold metadata-write rights on both
the source and the destination containers.

(thumbnail-service-authentication-permissions-the-service-identity-needs)=
## Permissions the service identity needs

Every call the service identity makes — publishing an event, provisioning a queue, reading or writing a
storage object — is an authorized action on the *other* system's side, not something this chart enforces or
can grant on its own. Notification Service and Storage service are API contracts, not necessarily one fixed
product: your deployment might run NVIDIA's reference implementations, your own compatible services, or a
mix of both. Whichever concrete deployment answers each interface is what checks the service identity's
token against its own authorization policy at call time; a missing grant surfaces as `PERMISSION_DENIED`
(gRPC) or `403` (REST) from that system, not from Thumbnail Service. Configuring these grants is therefore an
operation on each downstream deployment's own administration surface, done by whoever operates it — not a
Helm value in this chart.

Enforcing the Notification Service's own authorization is optional, and — when a deployment turns it on — is
itself commonly backed by a separate Permission Service, which is likewise an interface a customer may
satisfy with their own compatible implementation rather than a specific required product. The policy shape it
expects is shown in
{ref}`Authenticating to the Notification Service <thumbnail-service-notifications-authenticating-to-the-notification-service>`.
The table below lists what the service identity needs on each system and why:

| System | Grant | Why |
|---|---|---|
| Notification Service (Event Aggregation) | `event-aggregation-service:publish-event` on `EventType:omni.thumbnail.generate` (and on the two association event types when their authorization is enforced) | The orchestrator publishes every generation and association event |
| Notification Service (Event Consumer) | `event-consumer-service:create-durable-queues`, `event-consumer-service:consume-durable-queues` | Both components provision their own durable queue at startup and consume it. Only the orchestrator tolerates a consume-only identity: on `PERMISSION_DENIED` it logs a warning and attaches to the presumed-existing queue. The worker aborts startup on the same status, so `create-durable-queues` is required in practice |
| Storage service | read on every watched container; write and metadata-write on every address `THUMBNAIL_PATH_TEMPLATE` can render; metadata-write on the watched containers; delete on rendered destinations | Source download, thumbnail upload, association metadata on both sides, and the storage-deleted sweep |

See {ref}`Authenticating to the Notification Service <thumbnail-service-notifications-authenticating-to-the-notification-service>`
for a concrete example of what a Notification Service permit looks like.

(thumbnail-service-authentication-references)=
## References

- [OAuth 2.0 Client Credentials Grant](https://datatracker.ietf.org/doc/html/rfc6749#section-4.4) — the grant type client-credentials mode implements
- [RFC 3339](https://datatracker.ietf.org/doc/html/rfc3339) — timestamp format required for `expires_on` in the access-token file
- [Microsoft Entra ID application ID URI and default scope](https://learn.microsoft.com/en-us/entra/identity-platform/scopes-oidc#the-default-scope) — the `<application-client-id>/.default` scope convention
- [Azure Workload Identity](https://azure.github.io/azure-workload-identity/docs/) — the platform the access-token-file mode is designed around

(thumbnail-service-authentication-see-also)=
## See also

- {ref}`Thumbnail destination path template <thumbnail-service-destination-path-template-thumbnail-destination-path-template>` — the next topic: how the service resolves where each generated thumbnail is written.
- {ref}`Environment configuration reference <thumbnail-service-configuration-environment-configuration-reference>` — every authentication variable, and the ones that were retired.
- {ref}`Thumbnail Service Configuration <thumbnail-service-deployment-thumbnail-service-configuration>` — full Helm chart walkthrough, including `serviceIdentity` in its wider context.
- {ref}`Service notifications <thumbnail-service-notifications-service-notifications>` — the primary consumer of the service identity's publish grants.
- {ref}`Association metadata and queries <thumbnail-service-associations-association-metadata-and-queries>` — how the caller's token and the service token divide the storage calls.
- {ref}`Troubleshooting <thumbnail-service-troubleshooting-troubleshooting>` — diagnosing `CrashLoopBackOff` at the token gate and `401` versus `403` responses.
