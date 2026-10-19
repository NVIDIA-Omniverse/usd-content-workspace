# Event Consumer Service — component map

> **Applies to:** Event Consumer Service · **Released:** 1.11.0 chart and image (usd-content-workspace Developer Guide 1.3.0 / NGC inventory) · **This repo's snapshot:** 1.12.0 (`pyproject.toml` `version`, `helm/event-consumer-service/Chart.yaml` `version`/`appVersion`, `values.yaml` `image.tag`; release commit "Release notifications-consumer-service 1.12.0"; the `CHANGELOG.md` `[Unreleased]` section carries the 1.12.0 changes) · **Sources:** [`notifications/reference/event-consumer-service/`](../../../../notifications/reference/event-consumer-service/), the [source skill package](../../NOTICE.md) upstream `event-consumer-service` skill

This page is a table of contents. Read the linked file before answering, and confirm the deployed version ([capture a deployment](../versions/capture-a-deployment.md)) before applying configuration.

## What it is

The consume side of the notification path. Clients open non-durable streams (ephemeral queues with a 20-second reconnect grace) or durable streams (RabbitMQ quorum queues that buffer across disconnects), over gRPC (`EventConsumerService`) or REST with Server-Sent Events. It authorizes each delivered event, manages durable queues, and gives operators a gRPC dead-letter triage stream. It requires RabbitMQ 4.x, and the chart applies a broker policy through the management API.

| Fact | Value (snapshot) |
|---|---|
| gRPC port | Chart `service.grpc.port: 50051`. Standalone process default `OMNI_EVENTS_GRPC_PORT=[::]:50052` (README, docs, and conformance examples use 50052). |
| REST port | `8000` (`service.rest.port`). REST is on in the chart (`service.rest.enabled: true`) and off for a standalone process (`OMNI_EVENTS_REST_API_ENABLED=False`). |
| Health | gRPC health (liveness service `liveness`; readiness reflects stream capacity and drain); REST `GET /api/v1beta/health` and `/api/v1/health`, unauthenticated |
| Capacity metrics | `GET /api/v1beta/metrics/channel-pool` (and `/api/v1/...`), unauthenticated, per pod, scraped by the chart's channel-pool autoscaler |
| Telemetry | OTLP/gRPC (`observability.*` or legacy `telemetry.*`) |
| Discovery | `ListServices` (`v2alpha`) unauthenticated on the gRPC port, type `notification-consumer`, REST entry `notifications-consumer` at `/api/v1beta`. Opt in with `service.annotations`. With `httpProxy.enabled`, the chart adds `discovery.omniverse.nvidia.com/external-rest-endpoint` and `external-grpc-endpoint` annotations, because the HTTPProxy routes through a separate `-envoy` Service. |
| Auth (snapshot chart) | `principalSource: ""`: the chart selects `jwt` when `openId.openIdConfigurationUri` and an audience (a `default` client registration or `openId.additionalJwtAudience`) are set, and otherwise fails the render. `jwtUnverified` renders only with `allowUnverifiedJwt: true` |
| REST CORS | `service.rest.corsAllowOrigins` (default `[]`, no CORS from the service; `*` and `null` fail the render). Separate from the ingress `httpProxy.corsPolicy` |
| Secret | `envSecretName` (default `event-consumer-env`): key `rabbitmq_uri`; key `redis_uri` only when `redis.enabled` |
| Policy hook | Post-install/post-upgrade Job applying RabbitMQ policy `event-queue-limits` (pattern `^event-queue:`, apply-to `quorum_queues`); needs a management-API Secret |
| Autoscaling | Own channel-pool autoscaler Deployment (`channelPoolAutoscaling.*`, 3–10 replicas by default), not an HPA |

## Route by task

| Question | Go to |
|---|---|
| Auth modes, token handling, queue-ID rules, reconnect tokens, filter-update acks, durable lifecycle, nack, consumer-loss errors | [`README.md`](../../../../notifications/reference/event-consumer-service/README.md) (top half) |
| Every env var and default | `README.md` → Configuration |
| Admission tiers, shed threshold, `X-Via-Proxy`, ingress retries | `README.md` → Capacity and admission tiers |
| Graceful drain and grace-period invariant | `README.md` → Graceful drain on scale-down and rollout |
| Broker sizing, HA, managed brokers, policy escape hatch | `README.md` → Production deployment — broker sizing and high availability |
| Consumer acknowledgement timeout | `README.md` → Bounding a hung durable consumer |
| Dead-letter queue bounds drift | `README.md` → Retuning the dead-letter queue's bounds |
| Permission gates, storage events, fine-grained YAML | `README.md` → Permissions; [`docs/permissions.md`](../../../../notifications/reference/event-consumer-service/docs/permissions.md) |
| Chart prerequisites and `policyHook` | [`helm/README.md`](../../../../notifications/reference/event-consumer-service/helm/README.md) |
| Chart values | [`helm/event-consumer-service/values.yaml`](../../../../notifications/reference/event-consumer-service/helm/event-consumer-service/values.yaml); local overrides [`helm/local-values.yaml`](../../../../notifications/reference/event-consumer-service/helm/local-values.yaml) |
| Templates (HTTPProxy streaming routes, envoy Service, autoscaler, hook Job) | [`helm/event-consumer-service/templates/`](../../../../notifications/reference/event-consumer-service/helm/event-consumer-service/templates/) |
| Policy-hook script | [`public_scripts/apply_queue_policy.py`](../../../../notifications/reference/event-consumer-service/public_scripts/apply_queue_policy.py) |
| Smoke tests (6: three gRPC, three REST) | [`helm/event-consumer-service/files/tests/`](../../../../notifications/reference/event-consumer-service/helm/event-consumer-service/files/tests/) |
| REST contract | [`openapi.yaml`](../../../../notifications/reference/event-consumer-service/openapi.yaml) |
| gRPC contract | [`.../consumer/v1beta/event_consumer.proto`](../../../../notifications/api/protos/nvidia/omniverse/notifications/consumer/v1beta/event_consumer.proto) |
| Customer docs for both services | [`docs/index.md`](../../../../notifications/reference/event-consumer-service/docs/index.md), [`getting-started.md`](../../../../notifications/reference/event-consumer-service/docs/getting-started.md), [`consuming-events.md`](../../../../notifications/reference/event-consumer-service/docs/consuming-events.md), [`publishing-events.md`](../../../../notifications/reference/event-consumer-service/docs/publishing-events.md), [`api-reference.md`](../../../../notifications/reference/event-consumer-service/docs/api-reference.md), [`permissions.md`](../../../../notifications/reference/event-consumer-service/docs/permissions.md); rendered `docs/docs.pdf` |
| Reimplementing from the contract | [`AGENTS.md`](../../../../notifications/reference/event-consumer-service/AGENTS.md) |
| Conformance | [`conformance_tests/README.md`](../../../../notifications/reference/event-consumer-service/conformance_tests/README.md), chart [`helm/event-consumer-conformance-tests/`](../../../../notifications/reference/event-consumer-service/helm/event-consumer-conformance-tests/) |
| Build from source: published `uv.lock`, `docker build --build-context common=../common .` | [`BUILDING.md`](../../../../notifications/reference/event-consumer-service/BUILDING.md), [`Dockerfile`](../../../../notifications/reference/event-consumer-service/Dockerfile), [`uv.lock`](../../../../notifications/reference/event-consumer-service/uv.lock) |
| History | [`CHANGELOG.md`](../../../../notifications/reference/event-consumer-service/CHANGELOG.md) |

Expert-tree pages: [deploy](../deploy/notifications.md) · [API](../apis/notifications.md) · [develop](../develop/custom-notifications-adapter/README.md) · [operate](../operate/notifications.md).

## Released 1.11.0 versus snapshot 1.12.0

From `CHANGELOG.md` `[Unreleased]` and the source. No wire-contract changes; authentication and CORS defaults change, plus behavior fixes and build and metadata changes. Later "Release notifications-consumer-service 1.12.0" commits re-published 1.12.0 with the security, build, OpenAPI-metadata, and author rows below; they did not change the version number.

| Area | 1.11.0 (released) | 1.12.0 (this snapshot) |
|---|---|---|
| `principalSource` default | `jwtUnverified` | `""`: chart selects `jwt` when OpenID and an audience are configured, otherwise the render fails. `jwtUnverified` needs `allowUnverifiedJwt: true` (renders `OMNI_EVENTS_ALLOW_UNVERIFIED_JWT`). **Potentially breaking:** a deployment with OpenID and no `principalSource` moves to `jwt` on upgrade |
| Auth render guards | Bad combinations crash-looped the pod | Render fails for an unknown mode, `jwt`/`opaque` without their OpenID values, `userInfoService` without URI, `disabled` with `permissions.enabled`, `permissions.enabled` without `permissions.endpoint`, several `default` registrations, a non-list `additionalJwtAudience`, a non-boolean `allowUnverifiedJwt` |
| REST CORS | Every origin answered, with credentials | Only `service.rest.corsAllowOrigins` (default `[]`); `*` and `null` fail the render |
| Non-durable storage events with `OMNI_EVENTS_STORAGE_PERMISSIONS_ENABLED` | Events for a folder the Storage API lists as empty (first file in a new folder, the new folder's `dir_created`, delete of the last file) could be withheld | Delivered when the principal may list the folder; logs `storage listing did not authorize a directory; its storage events are dropped until the recheck` (INFO for 403, WARNING for 400/unexpected 404) with a `dropped_events` count |
| Broker drops the consumer while the service is acking or requeuing | Generic `Internal Server Error` | Classified consumer-loss error: gRPC `INTERNAL`, REST SSE `error` with status 500, message `Internal Server Error: The broker cancelled this stream's consumer. Reconnect to resume consuming.` |
| Storage API 401 during authorization | Storage API response body returned to the client | Body not returned |
| Failed DLQ replay | `ActionOutcome.detail` carried the exception text | `republish to the origin queue failed (<exception class>)` |
| `notification-common` | 6.12.0 | 7.0.0, from the sibling `../common` path source (`==7.0.0` pin; `uv.lock` records 7.0.0) |
| Build from the published folder | Earlier published source: no `uv.lock`, and the `Dockerfile` could not build from the folder | `uv.lock` published (134 packages, all from public PyPI; `notification-common` locked as `../common` at 7.0.0); `docker build --build-context common=../common .` builds, exporting with `uv export --locked` (a stale lock fails the build) and installing with `uv pip install --no-deps`. The builder installs `uv==0.12.5` with pinned hashes |
| OpenAPI governance metadata | Governance ID in a top-level `tags` entry (per `CHANGELOG.md`) | Governance ID in an `x-` extension under `info`; no top-level `tags` list. No route or schema change |
| Package `authors` | Earlier published source: an individual | `NVIDIA` |

The 1.10.0 and 1.11.0 changes that both columns share (quorum queues, `ConsumeDurableEventsWithAcks`, DLQ, removal of `ack-durable`, deployment-wide `queue_id`, `nack_delivery_tags`, filter-update acks, `policyHook`, `shutdown`, `Last-Event-ID`) are covered in [operate](../operate/notifications.md#upgrade-path).

## Upstream material not in this snapshot

The upstream skill routed to `docs/sdd.md`, `docs/prd.md`, `SMOKE_TESTS.md`, `grafana-dashboard.json`, `CLAUDE.md`, the `docs/*-guide.md` contributor guides, and `notification_protos/`. None are in this repository; `values.yaml` comments also cite `SMOKE_TESTS.md` and SDD/PRD section numbers that you cannot follow here.

## Known defects in the in-repo docs

- `README.md` → Fine Grained Permissions shows policies with `Action::"event-aggregation-service:read-..."`. The consumer checks actions under its own `pyproject.toml` name (`notification_common.config.metadata.project_name()`), so policies must use `event-consumer-service:<action>`, as [`docs/permissions.md`](../../../../notifications/reference/event-consumer-service/docs/permissions.md) does.
- `values.yaml` `openId.principalIdClaim` says changing the claim orphans durable queues; that stopped being true in 1.11.0 (the README is correct).
- `values.yaml` `permissions.endpoint` says checks are disabled when the endpoint is unset, and `tests.auth` says auth is enabled by `openId.enabled`. Enforcement is keyed on `permissions.enabled` and authentication on `principalSource`.
- `CHANGELOG.md` 1.11.0 names `POST /api/v1beta/events/durable-queue`; the route is `POST /api/v1beta/queues/durable`.
- `helm/README.md` creates `event-consumer-env` with two separate `kubectl create secret generic` commands; the second fails with AlreadyExists. Pass both `--from-literal` flags in one command.
- `helm/README.md` lists a "direct exchange for notifications" prerequisite; the service declares its own topic exchange `events`.
- `AGENTS.md` says the conformance suite does not cover `ConsumeDurableEventsWithAcks`; the suite now covers its settlement protocol (not outcomes). Neither covers `ManageDeadLetteredEvents`.
- `conformance_tests/README.md` examples pin image and chart `1.9.9`; substitute the version you are testing.
- `README.md` and `AGENTS.md` name a repository other than this public one as the published home, and `helm/README.md` links the aggregation service's RabbitMQ notes through an absolute link into that repository. In this tree they are at [`../event-aggregation-service/README.md`](../../../../notifications/reference/event-aggregation-service/README.md) and its `helm/rabbitmq/` manifests.
- `CHANGELOG.md` `[Unreleased]` still says `BUILDING.md` explains that the `Dockerfile` does not build as-is; the `Fixed` entry above it and `BUILDING.md` now say it builds with `--build-context common=../common`.
- `docs/permissions.md` headings say `storage.file.created` and similar; the event types are `omni.storage.created`, `omni.storage.deleted`, `omni.storage.dir_created`, `omni.storage.dir_deleted`.
