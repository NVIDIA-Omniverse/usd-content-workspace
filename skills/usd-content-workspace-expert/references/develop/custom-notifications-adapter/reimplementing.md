# Reimplementing a notification service

> **Applies to:** Protocol-compatible replacements for the Event Aggregation or Event Consumer Service · **Released:** Aggregation 1.6.0 / Consumer 1.11.0 (contracts `1.0.0-beta.3`) · **This repo's snapshot:** Aggregation 1.7.0 / Consumer 1.12.0; conformance suites `0.0.0` in their `pyproject.toml` · **Sources:** [aggregation `AGENTS.md`](../../../../../notifications/reference/event-aggregation-service/AGENTS.md), [consumer `AGENTS.md`](../../../../../notifications/reference/event-consumer-service/AGENTS.md), both `conformance_tests/README.md`, both `BUILDING.md`, [`notification-common`](../../../../../notifications/reference/common/README.md)

Each service folder carries an `AGENTS.md` written for this task. The path is: read the `.proto` and `openapi.yaml`, implement gRPC and/or REST in your language, then run the black-box conformance suite against your service until it passes. You do not need RabbitMQ: the suites talk only to your service. You do not need the reference code either, but it shows every behavior the contract describes.

## What "compatible" requires

**Publisher** (`EventPublishingService`, REST `/api/v1beta/events[/batch]`):

- Per-event results with `success`/`failure_reason` inside `OK`/`200`; broker failures are not error statuses. Batch results in request order.
- Limits from Aggregation 1.7.0: 1000 events, `message` ≤ 65536 bytes measured as Python `json.dumps` defaults, `event_type` ≤ 256 and `resource_id` ≤ 2048 code points, REST `occurred_at` ≤ 64; whole batch refused with `422` / `INVALID_ARGUMENT` naming the field. Measure message size the same way so both implementations accept the same messages.
- Refusal order: size bound first (REST `413` before reading the body, with `Connection: close`; gRPC transport `RESOURCE_EXHAUSTED`), then authentication (`401` even if the body is also malformed), then request validation, then permission (`403`).
- Health at `GET /api/v1beta/health` → `{"status": "healthy"}`.

**Consumer** (`EventConsumerService`, REST `/api/v1beta/...`):

- SSE framing: an initial `connected` event whose `id` is the reconnect token and whose data includes `reconnect_token`; `event` frames; `error` frames carrying status after the `200`.
- `queue_id` and reconnect-token validation rules and status codes; ensure-exists create with `created` and REST `200`/`201`; `NOT_FOUND` for missing queues; the consumer-loss `INTERNAL` message; filter-update acks if you set `supports_filter_update_acks`.
- Filter semantics (`EQ`, lazy, greedy, empty-`resource_id` rules) — the reference implementation maps these to topic-exchange bindings on hashed, dot-joined path segments ([`notification_common/rabbitmq/routing.py`](../../../../../notifications/reference/common/notification_common/rabbitmq/routing.py)). A different broker needs equivalent matching.
- The consumer's published contract also documents operator-visible behavior (bounded redelivery, DLQ, idle expiry). The conformance suite cannot observe these; match them deliberately if you claim drop-in parity.

Both: use the `OMNI_EVENTS_` environment prefix if you want drop-in Helm parity, answer gRPC health, and, for the v2 Discovery operator, serve `nvidia.omniverse.discovery.capabilities.v2alpha.CapabilitiesService/ListServices` (as the reference services do; the snapshot operator calls v2beta first and falls back to it) unauthenticated with type `notification-aggregation` or `notification-consumer` and a Service port named `grpc` ([discovery API](../../apis/discovery.md), [add your service](../add-your-service.md)).

## Running conformance

```bash
# Publisher suite
cd notifications/reference/event-aggregation-service/conformance_tests
uv sync
export CONFORMANCE_GRPC_ENDPOINT=localhost:50051
export CONFORMANCE_REST_ENDPOINT=http://localhost:8001   # 8000 for a chart deployment
uv run run-conformance-tests        # or ./run_tests.sh

# Consumer suite
cd notifications/reference/event-consumer-service/conformance_tests
uv sync
export CONFORMANCE_GRPC_ENDPOINT=localhost:50052          # 50051 for a chart deployment
export CONFORMANCE_REST_ENDPOINT=http://localhost:8000
uv run run-conformance-tests
```

Set `CONFORMANCE_AUTH_TOKEN` for an auth-enabled service, and `CONFORMANCE_AUTH_REQUIRED=true` to add the anonymous-request-is-`401` cases. Each suite also ships a `docker-compose.yml` (set `SERVICE_IMAGE` to your image) and a Helm chart (`helm/event-*-conformance-tests/`) for in-cluster runs; the aggregation conformance chart's defaults target port 8000 in this snapshot (they were 8001, which the Service does not expose). The README examples pin old image and chart versions (`1.0.0`, `1.9.9`); use your own.

Coverage limits — a green run does **not** prove:

| Suite | Not covered |
|---|---|
| Publisher | `403`, `413`, `429`, `500`; authorization internals |
| Consumer | `ManageDeadLetteredEvents`; ack/nack outcomes (only the settlement wire protocol of `ConsumeDurableEventsWithAcks` is checked — the consumer `AGENTS.md` claim that with-acks is not covered is out of date); authorization internals |

Gate merges on the conformance container in CI and add your own integration tests for the gaps. Cross-suite guidance: [test → conformance](../../test/conformance.md).

## Building the reference services

From each service folder: `uv sync`, `OMNI_EVENTS_PRINCIPAL_SOURCE=disabled uv run start`, `uv run test` (no broker needed for tests). notification-common 7.0.0 refuses to start in its default `jwtUnverified` mode without an explicit opt-in, so a tokenless local run must select `disabled`. Use it only on your own machine, never for a shared deployment; select `jwt` with `OMNI_EVENTS_OPENID_CONFIGURATION_URI` and `OMNI_EVENTS_JWT_AUDIENCES` to verify tokens. The services resolve `notification-common` from `../common` as a path source, so build from a full checkout.

Each service folder ships a `uv.lock` that resolves every third-party package from public PyPI and records `notification-common` as the `../common` directory at 7.0.0, the version checked out there and the version both `pyproject.toml` files pin. Per each `BUILDING.md`, `uv sync --locked` installs exactly that set; if `../common` holds another version, `uv lock --check` reports the lock out of date and plain `uv sync` re-locks.

A replacement that claims drop-in parity with the 7.0.0-based services should also match their startup refusals: no `jwtUnverified` without `OMNI_EVENTS_ALLOW_UNVERIFIED_JWT=true`, and REST CORS only for the origins in `OMNI_EVENTS_CORS_ALLOW_ORIGINS` (`*` and `null` refused). The conformance suites do not test either.

The published `Dockerfile` (builder `python:3.14-slim`, runtime `nvcr.io/nvidia/distroless/python:3.14-v4.1.4`, entrypoint `python -um app.main`) now builds from the service folder when you pass `../common` as a named BuildKit context:

```bash
docker build --build-context common=../common -t event-consumer-service .
```

It runs `uv export --locked` and `uv pip install --no-deps`, so the image gets exactly the locked third-party versions plus `notification-common` from `../common`; `--locked` fails the build when the lock is out of date with `pyproject.toml` or `../common`. This was read from the Dockerfiles, not built. The consumer builder installs `uv==0.12.5` with pinned hashes; the aggregation builder copies `ghcr.io/astral-sh/uv:latest`, which is not pinned. Python ≥ 3.14 for the services, ≥ 3.12 for the library. See [building from source](../../deploy/building-from-source.md).

If you extend the reference implementation instead of replacing it, reuse `notification-common` for auth, permission checks, channel pools, shutdown, and capabilities ([component map](../../components/notification-common.md)); a subclass enables Discovery by overriding `capabilities_service_entry()` (helper `build_capabilities_entry(...)`).
