# Deploy notifications (RabbitMQ, Event Aggregation, Event Consumer)

> **Applies to:** RabbitMQ broker, Event Aggregation Service, Event Consumer Service, Storage Service `notificationClient` · **Released:** Aggregation 1.6.0, Consumer 1.11.0, RabbitMQ chart 99.3.0 with image `4.1.3-debian-12-r1` (Guide 1.3.0 / NGC) · **This repo's snapshot:** Aggregation 1.7.0, Consumer 1.12.0 (`pyproject.toml`, `Chart.yaml`, `image.tag`); no RabbitMQ chart in the repo · **Sources:** [aggregation `helm/`](../../../../notifications/reference/event-aggregation-service/helm/README.md), [consumer `helm/`](../../../../notifications/reference/event-consumer-service/helm/README.md), both `values.yaml` files and templates, [consumer `README.md`](../../../../notifications/reference/event-consumer-service/README.md), [storage events reference](../../../../storage/reference/storage-service/docs/helm/storage-backend-events.rst), the [source skill package](../../NOTICE.md)

Explain by default; produce or run a deployment only when asked. Before applying anything, record the versions actually deployed ([capture a deployment](../versions/capture-a-deployment.md)), read the matching chart's `values.yaml`, and keep the deployment's existing broker, vhost, auth, and ingress overrides. The values shown here are this snapshot's chart defaults. Most match the released 1.6.0 / 1.11.0 charts as the source package documents them; the exceptions, from each `CHANGELOG.md` `[Unreleased]`, are the authentication default and guards (`principalSource`, `allowUnverifiedJwt`), service-level REST CORS (`rest.corsAllowOrigins` / `service.rest.corsAllowOrigins`), and the aggregation chart's `openId.additionalJwtAudience`. Each is marked below.

Notifications are optional. Deploy them when clients must react to content changes (Kit-based apps watching create/delete, the [Thumbnail Service](thumbnails.md) consuming durable storage events, dashboards). Where they sit in the full install order is in [production stack](production-stack.md); NGC access, image pull secrets, and the choice between released and self-built images are in [artifact access](artifact-access.md) and [building from source](building-from-source.md).

## Components and order

1. **RabbitMQ 4.x** with quorum queues, persistence, and a dedicated vhost — before either service.
2. **Permission Service** and IdP wiring, if authorization is enforced — see [permissions and auth](permissions-and-auth.md).
3. **Event Aggregation Service.**
4. **Event Consumer Service** — its policy hook needs the broker's management API at install time.
5. **Storage Service** with `config.notificationClient` and `config.storageEvents` — redeploy after the aggregation service exists.
6. Contour ingress, TLS, and Discovery registration for both services.

| Fact | Aggregation | Consumer |
|---|---|---|
| Chart in this repo | [`notifications/reference/event-aggregation-service/helm/event-aggregation-service/`](../../../../notifications/reference/event-aggregation-service/helm/event-aggregation-service/) | [`notifications/reference/event-consumer-service/helm/event-consumer-service/`](../../../../notifications/reference/event-consumer-service/helm/event-consumer-service/) |
| Released chart (NGC) | `event-aggregation-service-1.6.0.tgz` | `event-consumer-service-1.11.0.tgz` |
| Image | `nvcr.io/nvidia/omniverse/event-aggregation-service` | `nvcr.io/nvidia/omniverse/event-consumer-service` |
| Kubernetes Service name | `event-aggregation-service` (chart name; `service.nameOverride` changes it) | `event-consumer-service`, plus `event-consumer-service-envoy` when `httpProxy.enabled` |
| Ports | gRPC `service.port` 50051, REST `rest.port` 8000 | gRPC `service.grpc.port` 50051, REST `service.rest.port` 8000 |
| Secret (`envSecretName`) | default `event-aggregation-env`, key `rabbitmq_uri` | default `event-consumer-env`, key `rabbitmq_uri`, plus `redis_uri` if `redis.enabled` |
| Replicas | `replicaCount: 1`, optional HPA (`autoscaling.*`, off) | channel-pool autoscaler on, `minReplicas: 3`, `maxReplicas: 10` |

The snapshot charts set `image.tag` to `1.7.0` / `1.12.0`. Those tags are not part of the released inventory; check that they exist in your registry before installing the snapshot charts, or set `image.tag` to the released version deliberately and accept the chart/image skew.

## RabbitMQ

Requirements, from the consumer README and chart:

- **RabbitMQ 4.x.** Durable queues are quorum queues (Consumer ≥ 1.10.0). The released inventory pins `4.1.3-debian-12-r1`; the consumer README recommends **4.1.5 or newer** because 4.0.5–4.1.4 has a policy-repair bug (`rabbitmq-server#14202`) when the delivery limit is negative. The chart refuses a negative `deliveryLimit`, so 4.1.3 is safe at chart settings, but upgrading the broker is a separate validated change. `quorum_queues` policy scoping needs ≥ 3.12.
- **Persistence on**, otherwise a broker restart loses every durable queue and message.
- **Three-node cluster for HA.** Size per the README's broker memory formula ([operate → capacity](../operate/notifications.md#capacity-and-scaling)). After adding a node, existing quorum queues do not gain replicas automatically; growing them (for example with `rabbitmq-queues grow`) is standard RabbitMQ administration, not documented in this repo.
- **A vhost shared by both services**, an AMQP user per service if you want separate credentials, and the **management HTTP API reachable from the cluster** for the consumer's policy hook.
- The services declare what they need: the durable topic exchange `events`, the dead-letter exchange `events.dlx`, and queue `events.dlq`. Both helm READMEs' "direct exchange for notifications" prerequisite is stale. The AMQP users therefore need configure, write, and read on the vhost; the dev definitions grant `.*` for all three.

This repository ships no RabbitMQ chart, only dev manifests for the RabbitMQ cluster operator in [`event-aggregation-service/helm/rabbitmq/`](../../../../notifications/reference/event-aggregation-service/helm/rabbitmq/) (single replica, not production ready, fixed password hashes — throwaway clusters only). The released stack uses NGC chart `rabbitmq-99.3.0`. The source package's values for that chart (`auth.existingPasswordSecret`, `loadDefinition`, `extraSecrets`, `initScripts`, `image.repositoryPrefix`) are unverified in this snapshot; inspect the pulled chart's `values.yaml`. The source package notes that chart 99.3.0 advertises `appVersion` 4.0.1 while its default image tag is `4.1.3-debian-12-r1`, and that it defaults to a staging repository prefix, so set `image.registry`, `image.repositoryPrefix`, `image.repository`, and `image.tag` explicitly and inspect rendered images.

Managed brokers: only dedicated tiers (management API available, channel and message caps above the services' defaults, no provider policy on the vhost) are supported.

## Secrets

Never put broker or management passwords in values files or shell history. Create Secrets from files or a secret manager (the charts also support the Secrets Store CSI driver via `secrets.csi.*`):

```bash
# AMQP URI: amqp://<user>:<password>@<host>:5672/<vhost>  (URL-encode the vhost; "/" is %2F)
kubectl create secret generic event-aggregation-env -n "$NAMESPACE" \
  --from-file=rabbitmq_uri=./aggregation-amqp-uri.txt
kubectl create secret generic event-consumer-env -n "$NAMESPACE" \
  --from-file=rabbitmq_uri=./consumer-amqp-uri.txt
# add --from-file=redis_uri=... in the same command if redis.enabled

# Management-API credential for the consumer's policy hook (deploy-time only)
kubectl create secret generic event-consumer-rabbitmq-mgmt -n "$NAMESPACE" \
  --from-file=username=./mgmt-user.txt --from-file=password=./mgmt-password.txt
```

Both charts read the same key name, so a single shared Secret (the source package used `event-services-env`) works via `envSecretName`, at the cost of one credential for both services. The management credential needs the `policymaker` tag and access to the vhost, not `administrator`; the running consumer never receives it.

## Authentication and authorization

`principalSource` (both charts) is the only switch for authentication. Pick it from your trust boundary, and use the same choice and the same `openId.principalIdClaim` (default `sub`; `oid` for Microsoft Entra ID) on both services so a caller resolves to one identity. Set it explicitly; the default differs by version:

- **Released 1.6.0 / 1.11.0:** default `jwtUnverified`.
- **Snapshot 1.7.0 / 1.12.0 (unreleased, notification-common 7.0.0):** default `""`. Left empty, the chart selects `jwt` when `openId.openIdConfigurationUri` and at least one audience are configured, and otherwise fails the render with a message naming both fixes. The chart never selects `jwtUnverified` by itself.

| `principalSource` | Token check | Use when |
|---|---|---|
| `jwt` | Validates against the IdP JWKS | Direct or untrusted clients, and the snapshot's choice when OpenID is configured. Needs `openId.openIdConfigurationUri`, an audience, and pod egress to the IdP discovery and JWKS endpoints |
| `jwtUnverified` | Decodes the JWT; **no** signature, audience, or expiry check | Only when every path to the pods verifies tokens first: the Envoy auth extension on the HTTPProxy **and** a NetworkPolicy (or equivalent) that keeps the ClusterIP Service unreachable directly, since in-cluster callers bypass the extension. Snapshot charts render it only with `allowUnverifiedJwt: true` (unquoted boolean), which sets `OMNI_EVENTS_ALLOW_UNVERIFIED_JWT=true`; without it the service refuses to start |
| `opaque` | IdP userinfo endpoint | Opaque tokens; needs `openId.openIdConfigurationUri` |
| `userInfoService` | `GET {userInfoService.uri}/api/v1alpha/users/me` | Principals from the [User Info Service](userinfo.md); default URI `http://userinfo-service:8080` |
| `disabled` | none | Local or test runs only, never shared deployments; incompatible with `permissions.enabled` |

Audiences: the consumer renders `OMNI_EVENTS_JWT_AUDIENCES` (for `jwt`/`opaque`) from the `default` client registration's `clientId` plus `openId.additionalJwtAudience`. The snapshot aggregation chart does the same (`openId.additionalJwtAudience`, default `[]`, is new in 1.7.0). Released aggregation 1.6.0 reads only the `default` client's `clientId`, so on 1.6.0 an API identifier issued as `aud` must go there.

Snapshot render guards: the charts fail `helm template` / `helm upgrade`, instead of crash-looping, for an unknown `principalSource`, `jwt` or `opaque` without their OpenID values, `userInfoService` without `userInfoService.uri`, `disabled` with `permissions.enabled`, `permissions.enabled` without `permissions.endpoint`, more than one `default` client registration, a non-list `additionalJwtAudience`, and a non-boolean `allowUnverifiedJwt`.

Authorization is separate and off by default:

| Gate | Chart value | What it enforces |
|---|---|---|
| Permission Service | `permissions.enabled` + `permissions.endpoint` (default `http://permission-service:3000`) on both charts | Aggregation: `publish-event` per `EventType`. Consumer: durable-queue create/update/consume/delete, DLQ management, durable storage events, fine-grained event types |
| Storage permissions | consumer `permissions.storage_permissions.enabled` + `.endpoints` (default `["http://storage-service:8012"]`; only the first is used) | Non-durable storage events, checked by listing the folder with the caller's token |
| Fine-grained events | consumer `permissions.content` (YAML `event_permissions: [...]`), mounted at `permissions.yaml_file_path` | Per-event-type action on `EventResourceId` |

Turn both consumer gates on to enforce both paths. Grant durable-queue and DLQ actions to service accounts only: they are deployment-wide. Policy syntax and examples: [develop → permissions](../develop/custom-notifications-adapter/permissions.md); Permission Service deployment: [permissions and auth](permissions-and-auth.md).

## Values overlays

Aggregation (merge into your values; keep existing overrides):

```yaml
image:
  pullSecrets:
    - name: <ngc-pull-secret>
envSecretName: event-aggregation-env
principalSource: jwt
openId:
  openIdConfigurationUri: "https://<idp>/.well-known/openid-configuration"
  principalIdClaim: "sub"            # identical on the consumer
  clientRegistrations:
    - name: default
      clientId: "<client-id>"        # becomes the accepted JWT audience
  additionalJwtAudience: []          # snapshot 1.7.0 only; not in the released 1.6.0 chart
permissions:
  enabled: true
  endpoint: "http://permission-service:3000"
service:
  annotations:
    discovery.omniverse.nvidia.com/enabled: "true"
observability:
  metrics:  { enabled: true, endpoint: { grpc: "http://<otel-collector>:4317" } }
  tracing:  { enabled: true, endpoint: { grpc: "http://<otel-collector>:4317" } }
```

Consumer:

```yaml
image:
  pullSecrets:
    - name: <ngc-pull-secret>
envSecretName: event-consumer-env
principalSource: jwt
openId:
  openIdConfigurationUri: "https://<idp>/.well-known/openid-configuration"
  principalIdClaim: "sub"
  clientRegistrations:
    - name: default
      clientId: "<client-id>"
  additionalJwtAudience: []          # add API identifiers here if aud is not the client id
permissions:
  enabled: true
  endpoint: "http://permission-service:3000"
  storage_permissions:
    enabled: true
    endpoints:
      - "http://storage-service:8012"
policyHook:                           # required: the chart refuses to render without these three
  managementApi:
    baseUrl: "http://<rabbitmq-service>.<namespace>.svc.cluster.local:15672"   # never assume 15672 on a managed broker
    vhost: "<vhost>"                  # must equal the vhost in rabbitmq_uri; the Job checks
    existingSecret:
      name: event-consumer-rabbitmq-mgmt
service:
  annotations:
    discovery.omniverse.nvidia.com/enabled: "true"
```

Leave `service.rest.enabled: true` on the consumer: the channel-pool autoscaler scrapes the REST metrics endpoint. Do not attach an HPA to the consumer Deployment as well.

Upgrading an existing consumer from before 1.11.0: the three `policyHook.managementApi.*` values and the Secret must exist **before** the next `helm upgrade`, or the render or the hook fails the release. `shutdown.preStopDelaySeconds` is rejected; use `shutdown.settleDelaySeconds`. See [operate → upgrade path](../operate/notifications.md#upgrade-path).

## Ingress and TLS

Both charts render Contour `HTTPProxy` objects when `httpProxy.enabled`: REST at `<httpProxy.fqdn.host>.<httpProxy.fqdn.domain>` and gRPC (h2c upstream) at `grpc.<host>.<domain>`. Certificates therefore need `event-aggregation.<domain>`, `grpc.event-aggregation.<domain>`, `event-consumer.<domain>`, `grpc.event-consumer.<domain>` (or wildcards `*.event-aggregation.<domain>`, `*.event-consumer.<domain>`). TLS: `httpProxy.tls.enabled` + `secretName`, or `httpProxy.tls.certManager.*`. Both set HSTS by default. The ingress CORS policy (`httpProxy.corsPolicy`) defaults to `allowOrigin: ["*"]` without credentials; restrict it for browser clients.

The services' own REST CORS is separate. In the snapshot charts (unreleased, notification-common 7.0.0) it is set by aggregation `rest.corsAllowOrigins` and consumer `service.rest.corsAllowOrigins`, rendered as `OMNI_EVENTS_CORS_ALLOW_ORIGINS`. The default `[]` adds no CORS middleware. Listed origins are echoed with credentials, so write each exactly as a browser sends `Origin` (`https://app.example.com`: scheme, ASCII host, no default port, no path or trailing slash). The charts refuse `*`, `null`, a non-list value, and non-string entries; the service refuses other malformed origins at startup. Browser clients that call a service's REST API directly, not through the HTTPProxy, need their origins listed. Released 1.6.0 / 1.11.0 answer every origin with credentials.

The Envoy auth extension is attached when `httpProxy.authExtension.enabled` (or legacy `openId.enabled`) is true. It validates only traffic through the HTTPProxy, so on its own it does not make `jwtUnverified` safe (see the table above). Wiring is in [permissions and auth](permissions-and-auth.md). Contour, cluster TLS, and shared `ingress-values.yaml` layering are in [production stack](production-stack.md).

Consumer streaming routes (SSE paths and the four streaming RPCs, including `ManageDeadLetteredEvents`) have hardcoded 240-minute timeouts (SSE idle 40 s) and their own retry and load-balancing policy (`httpProxy.streaming.*`); unary calls use `httpProxy.timeoutPolicy` (10 s) and `httpProxy.unary.retryPolicy`. Aggregation routes use `httpProxy.timeoutPolicy` (15 s) and carry no retry policy. Requires Contour ≥ 1.14 for the retry fields. Preserve these routes if you front the services with something other than the charts' HTTPProxy, and never let clients set `X-Via-Proxy` themselves.

Cloud L4 load balancers (AWS NLB/ALB, Azure LB) cut idle long-lived streams after 60 s–4 min regardless of gRPC keep-alive; clients must reconnect (see [API → error contract](../apis/notifications.md#consumer-error-contract)).

## Discovery registration

Both services answer `CapabilitiesService.ListServices` on their gRPC port (unauthenticated; `OMNI_EVENTS_GRPC_CAPABILITIES_ENABLED` default true). Add `discovery.omniverse.nvidia.com/enabled: "true"` under `service.annotations` so the v2 Discovery operator picks them up; the Service port is already named `grpc`. With ingress enabled, the consumer chart also stamps `external-rest-endpoint` and `external-grpc-endpoint` annotations derived from `httpProxy.fqdn`. Operator behavior is in [discovery](discovery.md).

## Storage Service integration

The Storage Service publishes `omni.storage.*` events through the aggregation gRPC API. Configure it in the storage chart (owned by [production stack](production-stack.md); full reference [`storage-backend-events.rst`](../../../../storage/reference/storage-service/docs/helm/storage-backend-events.rst)):

```yaml
config:
  storageEvents:                      # subscriber ID -> sqs or azureServiceBus source; see the storage reference
    <subscriber-id>: { ... }
  notificationClient:
    enabled: true
    endpointUrl: "http://event-aggregation-service.<namespace>.svc.cluster.local:50051"
    batchSize: 100                    # per BatchPublishEvents call; must stay <= 1000
```

Without `notificationClient.credentials`, the client authenticates with the Storage Service's `serviceIdentity`; `credentials.accessTokenFile.path` delegates token acquisition to a sidecar. Backend SQS or Service Bus credentials do not authenticate publishing. Grant that principal `event-aggregation-service:publish-event` on each of the four event types (or the custom names under `notificationClient.eventTypes`). Synthetic `dir_created`/`dir_deleted` events (`config.syntheticDirEvents`, on by default, 6 s flush) are best-effort. An event subscriber does nothing unless caches or the notification client are enabled.

## Install and verify

```bash
# From each service folder in this repo, or from the pulled released chart directory
helm lint ./helm/event-aggregation-service -f aggregation-values.yaml
helm template ./helm/event-consumer-service -f consumer-values.yaml >/tmp/consumer.yaml   # inspect images, env, guards

helm upgrade --install event-aggregation-service ./helm/event-aggregation-service \
  -n "$NAMESPACE" -f aggregation-values.yaml
helm upgrade --install event-consumer-service ./helm/event-consumer-service \
  -n "$NAMESPACE" -f consumer-values.yaml

kubectl -n "$NAMESPACE" rollout status deploy -l app.kubernetes.io/name=event-aggregation-service
kubectl -n "$NAMESPACE" rollout status deploy -l app.kubernetes.io/name=event-consumer-service
kubectl -n "$NAMESPACE" get jobs | grep -i policy   # hook Job; logs kept 900 s (policyHook.ttlSecondsAfterFinished)
```

Render guards that fail `helm template`/`upgrade` on purpose: `terminationGracePeriodSeconds < settleDelaySeconds + drainWindowSeconds + 10`; observability enabled without `endpoint.grpc`; consumer `policyHook` missing its three inputs, `deliveryLimit < 0` or null, `messageTtlMs`/`maxLength` null or 0, `consumerTimeoutMs < 60000`. The snapshot charts add the authentication and CORS guards above.

Then:

1. Confirm the effective broker policy: `rabbitmqctl list_policies -p <vhost>` (or the management UI) shows `event-queue-limits`, pattern `^event-queue:`, apply-to `quorum_queues`, and your bounds.
2. Health: `GET /api/v1beta/health` on port 8000 of each Service.
3. Smoke tests: set `tests.enabled: true` with `tests.auth` (client credentials via `valueFrom.secretKeyRef`) or `tests.serviceIdentity.accessTokenFile`, then `helm test <release>`. Aggregation runs 4 tests and needs `publish-event` on `omni.storage.created` and `test.batch.publish`; consumer runs 6. Set `smokeTest.image.tag` to the smoke-test image version your inventory provides (the charts default to `1.0.1`; the source package cites `1.0.9` for the released inventory).
4. End-to-end: subscribe, then publish (tokens from your IdP, held in environment variables):

```bash
kubectl -n "$NAMESPACE" port-forward svc/event-consumer-service 8000:8000 &
kubectl -n "$NAMESPACE" port-forward svc/event-aggregation-service 8001:8000 &

FILTERS='[{"event_type":"omni.storage.created","filters":[{"filter_type":"starts_with_greedy","resource_id":""}]}]'
curl -N -G "http://localhost:8000/api/v1beta/events/stream" \
  -H "Authorization: Bearer $CONSUMER_TOKEN" --data-urlencode "filter_groups=$FILTERS"

curl -X POST "http://localhost:8001/api/v1beta/events" \
  -H "Authorization: Bearer $PUBLISHER_TOKEN" -H "Content-Type: application/json" \
  -d '{"event":{"event_type":"omni.storage.created","message":{"probe":true},"occurred_at":"2026-01-01T00:00:00Z","resource":{"resource_id":"/probe/a.txt"}}}'
```

Expect `event: connected`, then `event: event`. Check `result.success` on the publish response, not just HTTP 200. With `permissions.storage_permissions.enabled`, a non-durable subscriber receives a storage event only if its token can list the folder, so a synthetic resource like `/probe/` may be filtered out; use a real storage path or a custom event type for the probe.

Rollout, health, and smoke tests do not prove nack, reconnect, expiry recovery, or DLQ behavior; test those separately ([operate](../operate/notifications.md#verification)). Conformance suites: [test → conformance](../test/conformance.md).

## Local and from-source runs

`uv sync`, then `OMNI_EVENTS_PRINCIPAL_SOURCE=disabled uv run start` in each service folder, with RabbitMQ from `docker run -d -p 5672:5672 rabbitmq:4.1.3`, per each `BUILDING.md`. With notification-common 7.0.0 the built-in default `jwtUnverified` is refused at startup, so a tokenless run must select `disabled`. That is for a local run on your own machine only, never for a shared deployment; to verify tokens locally, set `OMNI_EVENTS_PRINCIPAL_SOURCE=jwt` with `OMNI_EVENTS_OPENID_CONFIGURATION_URI` and `OMNI_EVENTS_JWT_AUDIENCES`. The published locks record the same `notification-common` 7.0.0 as `../common`, so per `BUILDING.md` `uv sync --locked` installs exactly the locked set (not run here). The aggregation REST port is 8001 under `uv run start`; the consumer needs `OMNI_EVENTS_REST_API_ENABLED=true` for REST and listens on gRPC 50052. The consumer's [`helm/local-values.yaml`](../../../../notifications/reference/event-consumer-service/helm/local-values.yaml) sets `principalSource: disabled` and a local image. Cluster-based local loops: [local development](local-development.md).

To build your own images from this snapshot, run `docker build --build-context common=../common -t <registry>/<service>:<tag> .` in each service folder (BuildKit required). Each folder now ships a `uv.lock` resolved from public PyPI; the image installs exactly those third-party versions with `uv export --locked`, plus `notification-common` from `../common`. Because the export is `--locked`, a lock that is out of date with `pyproject.toml` or with `../common` fails the build. Earlier imports shipped no lock and their `Dockerfile`s could not build from the published folder. Images you build this way are not the released NGC images; record their digests and scan them yourself. Details: [develop → reimplementing](../develop/custom-notifications-adapter/reimplementing.md#building-the-reference-services), [building from source](building-from-source.md).
