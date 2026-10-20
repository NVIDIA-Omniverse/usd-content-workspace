This is the helm chart for the Event Consumer Service.

# Prerequisites
## RabbitMQ
RabbitMQ should be set up in the cluster. The following are required for this service
- direct exchange for notifications
- user (with username and password) with permissions to create queues bound to this exchange
- dedicated vhost for notifications
- the management HTTP API reachable from the cluster, plus a credential that may set policies on that vhost — see [Durable-queue bounds policy (`policyHook`)](#durable-queue-bounds-policy-policyhook) below
- RabbitMQ 4.1.5 or newer is recommended (3.12 is the floor for the `quorum_queues` policy scoping the chart applies)

See the documentation for the [event aggregation service](../../event-aggregation-service) for how a dev rabbitmq cluster was set up in k8s.

## Secrets
### Registry Credentials
The images are stored in NGC. To create the expected imagePullSecret, run:
```
kubectl create secret docker-registry regcred --docker-server="nvcr.io" --docker-username='$oauthtoken' --docker-password=$NGC_TOKEN --namespace=$NAMESPACE
```
Where $NGC_TOKEN is an NGC API key with access to the image registry, and $NAMESPACE is the namespace where the Event Consumer Service will be deployed.

### Sensitive Environment Variables
Some environment variables are sensitive and stored in a secret. Currently, this includes the RabbitMQ URI and Redis URI, which may need to contain a username and password. To create the expected secret, run:
```
kubectl create secret generic event-consumer-env -n $NAMESPACE --from-literal=rabbitmq_uri=$RABBITMQ_URI
```

and

```
kubectl create secret generic event-consumer-env -n $NAMESPACE --from-literal=redis_uri=$REDIS_URI
```
Where $RABBITMQ_URI is the uri, in the format of `amqp://<username>:<password>@<host>:<port>/<vhost>`, and $NAMESPACE is the namespace where the Event Consumer Service will be deployed.

# Durable-queue bounds policy (`policyHook`)

The chart ships the RabbitMQ policy that bounds durable event queues — redelivery limit, message TTL, maximum length, overflow behavior, and an idle-queue TTL that auto-deletes a durable queue with no attached consumer. A `post-install,post-upgrade` hook Job applies it through the broker's management HTTP API, so it lands on install and is re-asserted on every `helm upgrade`.

The Job is **fail-closed**: if the policy cannot be applied, the Helm release fails. That is deliberate — a deployment whose queues carry no bounds is the condition this hook exists to prevent.

## What a deployment must provide

The hook is enabled by default and has no defaults for the broker reach, because a wrong default here means "applied successfully to the wrong broker". With `policyHook.enabled: true`, the chart **refuses to render** unless all three of these are set — on a fresh install *and* on every `helm upgrade`, including the first upgrade of a release that predates this hook. An existing deployment that has not set them will fail to render on its next upgrade until it does:

| Value | What it is |
|---|---|
| `policyHook.managementApi.baseUrl` | Management API base URL including scheme, host and port, with no trailing path. Self-hosted is typically `http://<rabbitmq-service>.<namespace>.svc.cluster.local:15672`; a managed broker commonly serves it over HTTPS on 443, e.g. `https://<instance>.rmq.cloudamqp.com`. Never assume `:15672`. |
| `policyHook.managementApi.vhost` | The vhost the policy is applied to. It must be the same vhost the service consumes from — the Job parses the vhost out of the service's own AMQP URI and fails the release on a mismatch, because a policy applied to the wrong vhost otherwise succeeds silently and bounds nothing. |
| `policyHook.managementApi.existingSecret.name` | Name of an existing Secret in the release namespace holding the management credential. Keys default to `username` and `password`, overridable via `existingSecret.usernameKey` / `existingSecret.passwordKey`. |

The hook Job runs on upgrade as well as on install, and a missing Secret fails the release rather than being skipped. So create the credential Secret before installing — and, for a deployment already running an earlier chart version, **before your next `helm upgrade`**:

```bash
kubectl create secret generic event-consumer-rabbitmq-mgmt -n $NAMESPACE \
  --from-literal=username=$RABBITMQ_MGMT_USERNAME \
  --from-literal=password=$RABBITMQ_MGMT_PASSWORD
```

The credential needs the `policymaker` tag plus access to the vhost; `administrator` is only required for operator policies, which this chart does not use. It is a **deploy-time credential** held by the hook Job alone — the running service never receives it and keeps its own least-privilege AMQP credential from `event-consumer-env`.

## Values and defaults

Job shape:

| Value | Default | Notes |
|---|---|---|
| `policyHook.enabled` | `true` | See the escape hatch below. |
| `policyHook.backoffLimit` | `1` | Whole-pod retries — two pod attempts in total. Broker flakiness is not what this covers: the script retries each management-API call itself (3 attempts with backoff). This is for whole-pod failures — an image-pull error, a node eviction, an OOM kill. Raising it without raising `activeDeadlineSeconds` buys nothing, because the extra attempts are cut off mid-run. |
| `policyHook.activeDeadlineSeconds` | `180` | Wall-clock bound across *all* pod attempts, so a broker outage fails the release instead of hanging it. Sized for two worst-case pod attempts (~83 s each) plus Kubernetes' 10 s inter-attempt backoff. Keep it in step with `backoffLimit`; shortening it below ~90 s kills the pod as `DeadlineExceeded` before the script can log why the broker would not answer. |
| `policyHook.ttlSecondsAfterFinished` | `900` | Seconds before the finished Job is cleaned up; `0` keeps it so you can read its logs. Note that Helm does not remove hook resources on `helm uninstall`. |
| `policyHook.resources` | small requests/limits | The Job makes at most two HTTP calls and exits. |

The policy itself:

| Value | Default | Notes |
|---|---|---|
| `policyHook.policy.name` | `event-queue-limits` | Renaming it orphans the previous policy, which keeps applying until you delete it by hand. |
| `policyHook.policy.priority` | `0` | Raise only to win against another regular policy matching the same queues; equal priorities resolve nondeterministically. |
| `policyHook.policy.definition.deliveryLimit` | `10` | Must be `>= 0`; the chart refuses a negative value, which on RabbitMQ 4.0.5–4.1.4 triggers a repair loop that stops queues from ever expiring. |
| `policyHook.policy.definition.messageTtlMs` | `86400000` (24 h) | A message older than this is dead-lettered. |
| `policyHook.policy.definition.maxLength` | `100000` | Messages held per durable queue before overflow. |
| `policyHook.policy.definition.overflow` | `drop-head` | Keep this unless you have measured the alternative: `reject-publish` makes one full queue nack the publisher's confirms, marking other tenants' publishes on the same binding key as failed. |
| `policyHook.policy.definition.expiresMs` | `86400000` (24 h) | Idle-queue TTL. `null` omits the key entirely and disables auto-reap. `0` is not valid — the broker rejects it. Keep this at or above `messageTtlMs`. |

Two fields of the policy are deliberately **not** values, and are hardcoded in the template. Both answer *which queues the policy selects* — they are scoping, not tuning — and a changed value in either would silently apply the wrong bounds to the wrong queues with nothing failing anywhere:

- **The match pattern is fixed** at `^event-queue:`. It is derived from the service's queue-naming contract, and an operator who changed it would leave every event queue unbounded.
- **The apply-to scope is fixed** at `quorum_queues`. Durable and non-durable queues share the `event-queue:` name prefix, so the pattern alone cannot separate them — only the durable ones are quorum queues, which makes the queue type the whole of the separation. Widening the scope to `queues` would apply the maximum length and `drop-head` to non-durable queues, silently dropping a live-but-slow consumer's backlog — those queues have no dead-letter exchange, so the drops are invisible — and breaking the reconnect-token "no missed events" guarantee. `quorum_queues` scoping requires RabbitMQ 3.12 or newer.

## Operator notes

- **At most one regular policy applies to a queue** (highest priority wins; equal priorities resolve nondeterministically). A second regular policy matching `^event-queue:` **displaces** this one rather than merging with it, so keep every bound for these queues in this single policy.
- **The escape hatch is `policyHook.enabled: false`**, for operators on restricted brokers who manage policies themselves. What it costs depends on whether the policy was ever applied, and the two cases are not the same:
  - **Never applied** (a fresh cluster, or a broker where you removed the policy yourself): durable queues carry no maximum length, no message TTL and no idle-queue TTL, and only RabbitMQ 4.x's built-in redelivery-limit default of 20 bounds anything. This is the cliff — apply an equivalent policy by hand before running this way.
  - **Disabled after it had been applied**: nothing in this chart deletes the policy. The last-applied `event-queue-limits` policy **stays live on the broker and keeps enforcing**, including reaping idle durable queues, frozen at whatever values it last carried. Disabling the hook stops the chart from *updating* the policy; it does not remove it. To actually get rid of it, delete it yourself — `rabbitmqctl clear_policy -p <vhost> event-queue-limits`, or the Policies page in the management UI.
  - **Either way, do not add a second policy of your own alongside it.** A second regular policy matching the same queues **displaces** this one rather than merging with it (see the first note above), so an operator-applied policy meant to supplement a still-live `event-queue-limits` will silently replace it instead.
- **The idle-queue TTL changes durable-queue behavior for clients.** A durable queue with no attached consumer for `expiresMs` is deleted, and a returning client gets `NOT_FOUND` / `404` until it recreates the queue. See the Durable-queue lifecycle section of the service `README.md` before changing or disabling it.
- **An unchanged policy is not re-applied.** The Job reads the existing policy first and skips the write when it already matches, because applying a policy restarts the idle clock on every queue it matches — an unconditional write on a frequent deploy cadence would keep abandoned queues alive indefinitely.
- **Expect the first auto-reap one full idle window after the deploy that first applies the policy.** Applying it — on first install, and on any upgrade that changes a policy value — resets the idle clock on **every** queue it matches, including queues that have already been abandoned for weeks. Those queues are not reaped at the next tick; they are reaped `expiresMs` after the apply. So a cluster with a long-standing backlog of abandoned durable queues stays at its current queue count for a full window (24 h at the default) before the count starts falling. That is expected, not a broken hook.

# Deployment
The chart does not render with its default values. Every install needs the
`policyHook.managementApi.*` values described under
[What a deployment must provide](#what-a-deployment-must-provide) (or
`policyHook.enabled: false`) and an authentication configuration.

## Authentication
Either configure OpenID, so the service verifies bearer tokens itself, or set
`principalSource` explicitly:

- **OpenID (`jwt`).** Leave `principalSource` unset and set
  `openId.openIdConfigurationUri` (the identity provider's
  `/.well-known/openid-configuration` URL) plus at least one JWT audience: a
  `default` entry in `openId.clientRegistrations`, whose `clientId` is the
  audience, or `openId.additionalJwtAudience` (a list). The chart then selects
  `jwt`. The pods need egress to the identity provider's discovery and JWKS
  endpoints.
- **An explicit `principalSource`.** `disabled` turns authentication off, for
  a test install without tokens; it cannot be combined with
  `permissions.enabled`. `opaque` and `userInfoService` have their own
  required values.
- **`jwtUnverified`** reads the caller from the token without verifying it.
  It renders only with both `principalSource: jwtUnverified` and
  `allowUnverifiedJwt: true`, and is safe only when every path to the pods
  verifies tokens first: the HTTPProxy's auth extension covers only requests
  through the HTTPProxy, so the Service must also be unreachable directly
  (for example through a NetworkPolicy).

The chart fails the render, with a message naming the value to set, for every
authentication configuration the service would refuse at startup.

## CORS
`service.rest.corsAllowOrigins` lists the browser origins the service's own
CORS middleware admits, with credentials allowed; the default `[]` adds no
CORS. Write each origin exactly as a browser sends it, for example
`https://app.example.com`. `*` and `null` fail the render.

See `principalSource`, `allowUnverifiedJwt`, `openId`, and
`service.rest.corsAllowOrigins` in `values.yaml` for the details.

## Install
From the repository root, with the OpenID configuration and an existing
management-credential Secret:
```
helm install event-consumer ./helm/event-consumer-service -n $NAMESPACE \
  --set openId.openIdConfigurationUri=$OPENID_CONFIGURATION_URI \
  --set 'openId.clientRegistrations[0].name=default' \
  --set "openId.clientRegistrations[0].clientId=$CLIENT_ID" \
  --set policyHook.managementApi.baseUrl=$RABBITMQ_MGMT_URL \
  --set policyHook.managementApi.vhost=$RABBITMQ_VHOST \
  --set policyHook.managementApi.existingSecret.name=event-consumer-rabbitmq-mgmt
```
Where $OPENID_CONFIGURATION_URI is the identity provider's
`/.well-known/openid-configuration` URL, $CLIENT_ID is the JWT audience,
$RABBITMQ_MGMT_URL is the management API base URL, and $RABBITMQ_VHOST is the
vhost the service consumes from.
