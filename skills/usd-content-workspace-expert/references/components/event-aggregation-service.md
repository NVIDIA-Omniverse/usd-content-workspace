# Event Aggregation Service — component map

> **Applies to:** Event Aggregation Service (publisher) · **Released:** 1.6.0 chart and image (usd-content-workspace Developer Guide 1.3.0 / NGC inventory) · **This repo's snapshot:** 1.7.0 (`pyproject.toml` `version`, `helm/event-aggregation-service/Chart.yaml` `version`/`appVersion`, `values.yaml` `image.tag`; release commit "Release notifications-publisher-service 1.7.0"; the `CHANGELOG.md` `[Unreleased]` section carries the 1.7.0 changes) · **Sources:** [`notifications/reference/event-aggregation-service/`](../../../../notifications/reference/event-aggregation-service/), the [source skill package](../../NOTICE.md) upstream `event-aggregation-service` skill

This page is a table of contents. Read the linked file before answering a configuration question, and confirm the version the deployment actually runs ([capture a deployment](../versions/capture-a-deployment.md)) before applying a value.

## What it is

The publish side of the Omniverse notification path. Callers send events over gRPC (`EventPublishingService`) or REST (`/api/v1beta`); the service authenticates the caller, optionally checks the `publish-event` permission per event type, and publishes the event to the RabbitMQ topic exchange `events`, where the [Event Consumer Service](event-consumer-service.md) delivers it to subscribers. The Storage Service is the main built-in publisher (`omni.storage.*` events).

| Fact | Value (snapshot; read the chart before relying on it) |
|---|---|
| gRPC port | `50051` (`service.port`; `OMNI_EVENTS_GRPC_PORT` default `[::]:50051`) |
| REST port | `8000` (`rest.port`, `OMNI_EVENTS_REST_PORT`); `uv run start` defaults it to `8001` |
| Health | gRPC health (probes use services `liveness` and default); REST `GET /api/v1beta/health` |
| Metrics | OTLP/gRPC export only (`observability.*` or legacy `telemetry.*`); no scrape endpoint |
| Discovery | Answers `CapabilitiesService.ListServices` (`nvidia.omniverse.discovery.capabilities.v2alpha`) unauthenticated on the gRPC port, type `notification-aggregation`, REST entry `notifications-publisher` `v1beta` at `/api/v1beta`. Opt in with `service.annotations: {discovery.omniverse.nvidia.com/enabled: "true"}`. The Service port is named `grpc`, which the operator requires. |
| Auth (snapshot chart) | `principalSource: ""`: the chart selects `jwt` when `openId.openIdConfigurationUri` and an audience (`openId.additionalJwtAudience` or the `default` client registration's `clientId`) are set, and otherwise fails the render. `jwtUnverified` renders only with `allowUnverifiedJwt: true` |
| REST CORS | `rest.corsAllowOrigins` (default `[]`, no CORS from the service; `*` and `null` fail the render). Separate from the ingress `httpProxy.corsPolicy` |
| Secret | `envSecretName` (default `event-aggregation-env`), key `rabbitmq_uri` |
| Ingress | Contour `HTTPProxy`: REST at `<host>.<domain>`, gRPC (h2c) at `grpc.<host>.<domain>`; default host `event-aggregation` |

## Route by task

| Question | Go to |
|---|---|
| What it does, env vars, request-size and per-event limits, graceful shutdown, auth modes, permissions, principal claim | [`README.md`](../../../../notifications/reference/event-aggregation-service/README.md) |
| Chart prerequisites, secrets, install | [`helm/README.md`](../../../../notifications/reference/event-aggregation-service/helm/README.md) (see defects below) |
| Chart values and their comments | [`helm/event-aggregation-service/values.yaml`](../../../../notifications/reference/event-aggregation-service/helm/event-aggregation-service/values.yaml) |
| Chart templates (settings ConfigMap, Deployment render guards, HTTPProxy) | [`helm/event-aggregation-service/templates/`](../../../../notifications/reference/event-aggregation-service/helm/event-aggregation-service/templates/) |
| Dev-only RabbitMQ cluster-operator manifests | [`helm/rabbitmq/`](../../../../notifications/reference/event-aggregation-service/helm/rabbitmq/) |
| Post-install smoke tests (4: two gRPC, two REST) | [`helm/event-aggregation-service/files/tests/`](../../../../notifications/reference/event-aggregation-service/helm/event-aggregation-service/files/tests/), `tests.*` values |
| REST contract | [`openapi.yaml`](../../../../notifications/reference/event-aggregation-service/openapi.yaml) (generated; read-only) |
| gRPC contract | [`notifications/api/.../publisher/v1beta/event_publisher.proto`](../../../../notifications/api/protos/nvidia/omniverse/notifications/publisher/v1beta/event_publisher.proto); see [notification-protos](notification-protos.md) |
| Usage guides (publishing, permissions, API reference) | [`event-consumer-service/docs/`](../../../../notifications/reference/event-consumer-service/docs/index.md), especially [`publishing-events.md`](../../../../notifications/reference/event-consumer-service/docs/publishing-events.md) |
| Reimplementing the service from the contract | [`AGENTS.md`](../../../../notifications/reference/event-aggregation-service/AGENTS.md) |
| Conformance suite (local, docker-compose, Helm) | [`conformance_tests/README.md`](../../../../notifications/reference/event-aggregation-service/conformance_tests/README.md), chart [`helm/event-aggregation-conformance-tests/`](../../../../notifications/reference/event-aggregation-service/helm/event-aggregation-conformance-tests/) |
| Building from source: published `uv.lock`, `docker build --build-context common=../common .` | [`BUILDING.md`](../../../../notifications/reference/event-aggregation-service/BUILDING.md), [`Dockerfile`](../../../../notifications/reference/event-aggregation-service/Dockerfile), [`uv.lock`](../../../../notifications/reference/event-aggregation-service/uv.lock) |
| Version history | [`CHANGELOG.md`](../../../../notifications/reference/event-aggregation-service/CHANGELOG.md) |
| Shared library it is built on | [notification-common](notification-common.md) |

Expert-tree pages: [deploy](../deploy/notifications.md) · [API](../apis/notifications.md) · [develop](../develop/custom-notifications-adapter/README.md) · [operate](../operate/notifications.md).

## Released 1.6.0 versus snapshot 1.7.0

From `CHANGELOG.md` `[Unreleased]` (the 1.7.0 content) and the source. Later "Release notifications-publisher-service 1.7.0" commits re-published 1.7.0 with the security, build, OpenAPI-metadata, and author rows below; they did not change the version number.

| Area | 1.6.0 (released) | 1.7.0 (this snapshot) |
|---|---|---|
| `principalSource` default | `jwtUnverified` | `""`: chart selects `jwt` when OpenID and an audience are configured, otherwise the render fails. `jwtUnverified` needs `allowUnverifiedJwt: true` (renders `OMNI_EVENTS_ALLOW_UNVERIFIED_JWT`). **Potentially breaking:** a deployment with OpenID and no `principalSource` moves to `jwt` on upgrade |
| JWT audiences | `OMNI_EVENTS_JWT_AUDIENCES` from the `default` client registration only | New `openId.additionalJwtAudience` list, added to the `default` client's `clientId`; rendered only for `jwt` and `opaque` |
| Auth render guards | Bad combinations crash-looped the pod | Render fails for `jwt` without URI or audience, `opaque` without URI, `userInfoService` without URI, `disabled` with `permissions.enabled`, `permissions.enabled` without `permissions.endpoint`, several `default` registrations, an unknown mode, a non-list `additionalJwtAudience`, a non-boolean `allowUnverifiedJwt` |
| REST CORS | Every origin answered, with credentials | Only `rest.corsAllowOrigins` (default `[]`); `*` and `null` fail the render |
| Per-event limits | REST enforced the 1000-event batch and string-length limits; gRPC did not; neither capped `message` size | Both APIs refuse a batch over 1000 events, `message` over 65536 JSON bytes, `event_type` over 256 characters, `resource.resource_id` over 2048. REST `422`, gRPC `INVALID_ARGUMENT`; one oversize event refuses the whole batch. **Potentially breaking.** |
| gRPC missing `occurred_at` detail | `occurred_at must be specified` | `event.occurred_at must be specified` / `events[<index>].occurred_at must be specified` |
| `notification-common` | 6.12.0 (pinned) | 7.0.0, from the sibling `../common` path source (`==7.0.0` pin; `uv.lock` records 7.0.0) |
| Conformance chart defaults | `service.restEndpoint`/`healthEndpoint` on port `8001` | port `8000`; docker-compose sets `OMNI_EVENTS_REST_PORT=8001` and runs `rabbitmq:4.1.3` |
| Build from the published folder | Earlier published source: no `uv.lock`, and the `Dockerfile` could not build from the folder | `uv.lock` published (131 packages, all from public PyPI; `notification-common` locked as `../common` at 7.0.0); `docker build --build-context common=../common .` builds, exporting with `uv export --locked` (a stale lock fails the build) and installing with `uv pip install --no-deps`. The builder copies the unpinned `ghcr.io/astral-sh/uv:latest` |
| OpenAPI governance metadata | Governance ID in a top-level `tags` entry (per `CHANGELOG.md`) | Governance ID in an `x-` extension under `info`; no top-level `tags` list. No route or schema change |
| Package `authors` | Earlier published source: an individual | `NVIDIA` |

1.6.0 itself (shared by both columns) added the Discovery `ListServices` RPC, `service.annotations`, the `observability` block, `rabbitmq.channel_overflow_enabled`/`channel_wait_ms`, `rest.maxRequestBytes` (413 before auth), the `shutdown` block with `terminationGracePeriodSeconds: 60`, authentication-before-validation ordering, and removed the never-produced `503` from the REST spec.

## Upstream material not in this snapshot

The upstream skill (source skill package) routed to `docs/sdd.md`, `docs/prd.md`, `SMOKE_TESTS.md`, `grafana-dashboard.json`, `CLAUDE.md`, `helm/local-values.yaml`, `notification_protos/`, and four `docs/*-guide.md` contributor guides. None of these are in this repository. Use the README and chart comments for design intent, and the consumer's `docs/` for usage.

## Known defects in the in-repo docs

- `README.md` and `values.yaml` (`openId.principalIdClaim`) still say durable-queue names embed the hashed principal ID (`event-queue:{hashed_principal_id}:{queue_id}`). Since Consumer 1.11.0 the name is `event-queue:<queue_id>`; changing the claim changes authorization identity, not queue reachability.
- `helm/README.md` lists a "direct exchange for notifications" prerequisite and its dev definitions declare a `notifications` direct exchange. The services declare and use their own durable topic exchange named `events`.
- `helm/README.md` runs `kubectl apply -f rabbitmq-definitions-configmap.yaml` without the `rabbitmq/` directory, and says to install "from the repository root" with `./helm/...`; in this repository run it from `notifications/reference/event-aggregation-service/`.
- The dev `rabbitmq-definitions-configmap.yaml` ships fixed password hashes for `notification-publisher` and `notification-consumer`. Use it only on a throwaway cluster.
- The settings ConfigMap renders `OMNI_EVENTS_GRPC_HEALTH_CHECKING_ENABLE` (missing the final `D`); it has no effect, and the library default (`true`) applies.
- `values.yaml` says `autoscaling.targetCPUUtilizationPercentage` defaults to 80; the value is `70`.
- `AGENTS.md` lists a `docs/images/` directory that is not present.
- `README.md` and `AGENTS.md` name a repository other than this public one as the published home, and the README's links to the consumer's customer docs (`getting-started.md`, `publishing-events.md`, `docs.pdf`, and the rest) are absolute links into that repository instead of relative paths. Use [`../event-consumer-service/docs/`](../../../../notifications/reference/event-consumer-service/docs/index.md) in this tree.
- `CHANGELOG.md` `[Unreleased]` still says `BUILDING.md` explains that the `Dockerfile` does not build as-is; the `Fixed` entry above it and `BUILDING.md` now say it builds with `--build-context common=../common`.
- The HTTPProxy enables the Envoy auth extension when `openId.enabled` or `httpProxy.authExtension.enabled` is true, not when `principalSource` enables authentication as the values comment states.
