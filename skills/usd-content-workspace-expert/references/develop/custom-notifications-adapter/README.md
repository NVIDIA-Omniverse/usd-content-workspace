# Build on the Notifications APIs

> **Applies to:** Notifications publisher and consumer APIs; clients and reimplementations · **Released:** Aggregation 1.6.0 / Consumer 1.11.0; NGC contracts `notifications-aggregation-api` and `notifications-consumer-api` `1.0.0-beta.3` · **This repo's snapshot:** Aggregation 1.7.0 / Consumer 1.12.0 (`pyproject.toml`); proto packages `v1beta` · **Sources:** [`notifications/api/protos/`](../../../../../notifications/api/README.md), [`docs/`](../../../../../notifications/reference/event-consumer-service/docs/index.md), both services' `openapi.yaml` and `AGENTS.md`, the [source skill package](../../../NOTICE.md)

A "custom notifications adapter" is one of three things. Pick the page that matches:

| You are building | Read |
|---|---|
| A service that **publishes** its own events (or relays another system's) | [publishing](publishing.md), [permissions](permissions.md) |
| An application or service that **reacts** to events (UI, cache invalidation, thumbnailer, sync job) | [non-durable](consuming-non-durable.md) or [durable](consuming-durable.md), [permissions](permissions.md) |
| A **drop-in replacement** for the Event Aggregation or Event Consumer Service (another language or broker) | [reimplementing](reimplementing.md) |

Adding a brand-new service to the stack (Helm, Discovery annotations, auth) is covered generally in [add your service](../add-your-service.md). Exact wire behavior is in the [API reference](../../apis/notifications.md). Confirm the service versions you target ([capture a deployment](../../versions/capture-a-deployment.md)); several behaviors differ between released 1.6.0/1.11.0 and snapshot 1.7.0/1.12.0, and each page marks them.

## Architecture

```
 publisher ──gRPC/REST──▶ Event Aggregation ──AMQP──▶ RabbitMQ topic exchange "events"
 (your service,           Service :50051/:8000          │  per-subscriber queues
  Storage Service)                                       ▼  event-queue:<id> (durable = quorum)
                                                  Event Consumer ──gRPC/REST+SSE──▶ consumers
                                                  Service :50051/:8000
```

The two services share no state beyond the broker and vhost. Each authenticates callers itself (or relies on the Envoy auth extension in front of it) and checks the Permission Service when enabled. Events are routed by `event_type` and a hashed, slash-split `resource_id`, so filters on the consumer side become broker bindings rather than server-side scans.

## Concepts

- **Event** — `event_type` (string you define, `service.entity.action`, ≤ 256 characters, ≤ 5 dot parts), `message` (JSON object, ≤ 65536 bytes from Aggregation 1.7.0), `occurred_at` (UTC), optional `resource.resource_id` (slash-separated path, ≤ 2048). The services do not validate names or schemas; document your contract for consumers.
- **Consumed event** adds `principal_identity` (who published) and `published_at`; it does not include `resource_id`.
- **Built-in types** — `omni.storage.created`, `omni.storage.deleted`, `omni.storage.dir_created`, `omni.storage.dir_deleted`, published by the Storage Service. Only these exact names get storage permission handling; `myapp.file.created` is your own contract.
- **Non-durable** — a per-connection queue that lives 20 s after disconnect for reconnects. For live UI and dashboards.
- **Durable** — an explicitly created quorum queue that buffers across disconnects, bounded by message TTL (24 h), length (100000), delivery limit (10), and idle expiry (24 h without a consumer). For services that must process every event they can, plus source reconciliation for the rest: neither durability nor the dead-letter queue is lossless.
- **Filters** — per `event_type`, OR'd `EQ` / `STARTS_WITH_LAZY` (one level below the prefix) / `STARTS_WITH_GREEDY` (any depth). `STARTS_WITH_GREEDY` on `""` means all events of the type; `EQ` on `""` is an error.

## Generating clients

Generate from the protos in this repository (or from the NGC resource that matches your deployed service), not from the snippets in any guide:

```bash
cd notifications/api/protos
python -m pip install grpcio-tools          # bundles the well-known types (Struct, Timestamp)
python -m grpc_tools.protoc -I. --python_out=OUT --pyi_out=OUT --grpc_python_out=OUT \
  nvidia/omniverse/notifications/publisher/v1beta/event_publisher.proto \
  nvidia/omniverse/notifications/consumer/v1beta/event_consumer.proto
```

`buf generate` also works against the Buf module (`buf.yaml`, dependency `buf.build/protocolbuffers/wellknowntypes` from the public Buf Schema Registry, pinned in `buf.lock`); the plugin configuration used upstream is not published. Go and Java package options are set in the protos. For REST, generate from each service's `openapi.yaml` (OpenAPI 3.1), or call it directly. The services' own checked-in Python stubs are pinned to `grpcio-tools==1.76.0` for their smoke-test image; your client need not match.

Connect over TLS (`grpc.ssl_channel_credentials()`) through the ingress host `grpc.<host>.<domain>:443`, or in-cluster to the Service on port 50051. The examples in `docs/` use `:50052`, which is the consumer's standalone-process default, not the chart's.

## Client rules that apply everywhere

- Send `authorization: Bearer <token>` metadata (gRPC) or the `Authorization` header (REST). Load tokens from your credential provider or a mounted file; never embed them.
- Request a token whose audience the deployment accepts and whose configured principal claim (default `sub`) is non-empty — service accounts included.
- Expect long-lived streams to be cut: tokens expire mid-stream (`401` / `UNAUTHENTICATED`), L4 load balancers drop idle connections after 60 s–4 min (`UNAVAILABLE`), and pods drain on rollout (`503` / `RESOURCE_EXHAUSTED`). Reconnect with exponential backoff and jitter.
- Never send the `X-Via-Proxy` header; the chart's ingress adds it.
- Treat delivery as at-least-once with possible gaps: deduplicate on your side and reconcile from the source of truth after any expiry or `NOT_FOUND`.

## In-repo guides

The customer guides are thorough and match this snapshot: [getting started](../../../../../notifications/reference/event-consumer-service/docs/getting-started.md), [publishing](../../../../../notifications/reference/event-consumer-service/docs/publishing-events.md), [consuming](../../../../../notifications/reference/event-consumer-service/docs/consuming-events.md), [API reference](../../../../../notifications/reference/event-consumer-service/docs/api-reference.md), [permissions](../../../../../notifications/reference/event-consumer-service/docs/permissions.md). Their examples use illustrative names such as `storage.file.created`; the real storage types are `omni.storage.*`.
