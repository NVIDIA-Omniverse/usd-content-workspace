<!-- SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# AI Agent Guide for Reimplementing the NVIDIA Omniverse Event Aggregation Service

This document guides AI coding agents (Cursor, GitHub Copilot, Codex, and similar) and human engineers who want to build a protocol-compatible reimplementation of the NVIDIA Omniverse **Event Aggregation Service** from its published API contract.

The goal is concrete: read the API specifications, implement the gRPC and/or REST surface in the language of your choice, and prove compatibility by running the bundled conformance suite against your implementation until it passes.

## Project overview

The Event Aggregation Service accepts events from clients over gRPC or REST and publishes them to a message broker, where the separate **Event Consumer Service** delivers them to subscribers. It exposes two interchangeable API surfaces over the same publish operations:

- **gRPC** (primary) — the `EventPublishingService` contract.
- **REST/HTTP** (optional) — the same operations over plain request/response endpoints.

Both surfaces support publishing a single event or a batch of events in parallel:

- **Single publish** — submit one event and receive its publishing result.
- **Batch publish** — submit multiple events; the service publishes them into the broker in parallel and returns a per-event result for each.

This folder, `notifications/reference/event-aggregation-service` in the `usd-content-workspace` repository, holds the reference implementation together with the published API contract. All paths below are relative to this folder. The contract consists of:

- **API specifications** — the gRPC `.proto`, published once for all notification services under `notifications/api` (`../../api` from here), and the REST OpenAPI `openapi.yaml` in this folder.
- **A conformance suite** — a runnable, black-box test artifact that validates any implementation against the wire protocol.
- **Customer documentation** — the rendered manual and markdown pages for both notification services, published once under `notifications/reference/event-consumer-service/docs` (`../event-consumer-service/docs` from here).

The rest of the folder (`app/`, `helm/`, `tests/`, and the build files) is the reference implementation. You do not need it to build a compatible service, but it is a working example of every behavior the contract describes.

## The reimplementation journey

A compatible implementation follows three steps:

1. **Read the API specifications** — the `.proto` and `openapi.yaml` define the exact message shapes, RPCs, endpoints, status codes, and error semantics your implementation must honor.
2. **Run the conformance suite against your implementation** — the suite exercises the full API surface (happy paths and every documented error case) against a real running service, so a passing run is direct evidence of protocol compatibility.
3. **Iterate to compatibility** — each failing test names what was expected and what your service returned; fix and re-run until the suite is green.

## Key API-specification files

Read these before writing any code.

| File | Purpose |
|------|---------|
| `../../api/protos/nvidia/omniverse/notifications/publisher/v1beta/event_publisher.proto` | The gRPC `EventPublishingService` contract: single publish and parallel batch publish. The authoritative interface definition. |
| `../../api/protos/buf.yaml` | The buf module configuration for the proto tree. |
| `openapi.yaml` | The REST specification: paths, request/response schemas, and status codes. Generated from the service — treat it as read-only reference; do not hand-edit it. |
| `conformance_tests/` | The black-box conformance suite. See `conformance_tests/README.md` for how to run it. |
| `../event-consumer-service/docs/` | The customer documentation for both notification services (see *Customer documentation* below). |

## Step 1: Read the API contract

Start with `../../api/protos/nvidia/omniverse/notifications/publisher/v1beta/event_publisher.proto` — it is the primary surface. The `EventPublishingService` contract covers two unary RPCs:

- **`PublishEvent(PublishEventRequest) -> PublishEventResponse`** — publish a single event and receive its `PublishingResult` (the event, a `success` flag, and a `failure_reason` when it failed).
- **`BatchPublishEvents(BatchPublishEventsRequest) -> BatchPublishEventsResponse`** — publish one or more events; the service publishes them into the broker in parallel and returns a `PublishingResult` per event, in request order.

The message shapes are defined in the same `.proto`:

- **`Event`** — `event_type` (string), `message` (a `google.protobuf.Struct` JSON payload), `occurred_at` (a `google.protobuf.Timestamp`), and an optional `EventResource` whose `resource_id` is a slash-separated path string that consumers use for filtering.
- **`PublishingResult`** — the published `event`, a `success` boolean, and a `failure_reason` string populated only on failure.

A publish that fails at the broker is **not** signalled with an error status: both RPCs report it in the `PublishingResult` for the affected event — `success: false` with a `failure_reason` — inside an otherwise successful response.

The gRPC server listens on `[::]:50051`.

If you are also implementing the REST surface, read `openapi.yaml`. It maps the same publish operations onto HTTP. The port is `OMNI_EVENTS_REST_PORT` (default `8000`); `uv run start` defaults it to `8001`, the suite's default endpoint. The REST server mounts all routes under the `/api/v1beta` prefix. The endpoints the conformance suite exercises are:

- `POST /api/v1beta/events` — publish a single event.
- `POST /api/v1beta/events/batch` — publish a batch of events in parallel.
- `GET /api/v1beta/health` — health check.

## Status-code semantics

Return the documented status codes for each case; the gRPC status and its HTTP analogue map as follows:

| Condition | gRPC status | HTTP status | Client action |
|-----------|-------------|-------------|---------------|
| Bad or missing event data | `INVALID_ARGUMENT` | `400` | Fix the request; a batch with no events is rejected. |
| Request fails schema validation | `INVALID_ARGUMENT` | `422` | Fix the request payload. |
| Batch over 1000 events, an event `message` over 65536 bytes as JSON, `event_type` over 256 characters, or `resource_id` over 2048 characters | `INVALID_ARGUMENT` | `422` | Split the batch or shrink the event; one oversize event refuses its whole batch and nothing is published. |
| Request body over the size limit | `RESOURCE_EXHAUSTED` | `413` | Send a smaller body — split the payload or send fewer events per batch. |
| Missing / invalid / expired credentials | `UNAUTHENTICATED` | `401` | Supply valid authentication. |
| Authenticated but not permitted | `PERMISSION_DENIED` | `403` | The principal lacks permission to publish. |
| Batch rate limit exceeded | — | `429` | Back off and retry (batch endpoint). |
| Internal error | `INTERNAL` | `500` | Non-retriable without a fix server-side. |

The `413` is a resource bound, not an authorization decision: the REST surface refuses an oversize body **before** it authenticates or parses the request, so no event is published and the caller learns nothing about its own credentials from the refusal. The reference service sends the `413` with `Connection: close` and closes the connection, since the refused body is never read off the wire; a client must not reuse that connection. The reference service caps the body at 4 MiB by default (`OMNI_EVENTS_REST_MAX_REQUEST_BYTES`), chosen to match the gRPC transport's default inbound message cap so both surfaces refuse at the same size. The cap is **independent of the 1000-event batch limit** — either can refuse a request first, and for events averaging more than about 4 KiB the byte cap is the one that binds. On the gRPC surface the bound is applied by the transport rather than by service code: grpc's default inbound message limit rejects an oversize message with `RESOURCE_EXHAUSTED` before the RPC handler runs. That is now the only condition under which this API returns `RESOURCE_EXHAUSTED`, and it is a size bound — retrying the same message will not clear it.

The message size is measured as Python's `json.dumps` encodes the message with its defaults, which is how the reference service publishes it: `", "` and `": "` separators, every non-ASCII character escaped as `\uXXXX` (six bytes; twelve as a surrogate pair outside the Basic Multilingual Plane), plus JSON's usual escapes for quotes, backslashes, and control characters, and on gRPC the message converted from its `Struct` first, so every number is a double (`1` is measured as `1.0`). A reimplementation should measure the same way so the two accept the same messages; clients should leave headroom rather than size a message to the exact byte. The gRPC error names the field and, in a batch, the event's index (`events[1].message ...`), as REST's validation `loc` does. `event_type` and `resource_id` lengths count characters (Unicode code points). These per-event limits are separate from the 4 MiB transport-level size bound above: a gRPC request over it is refused with `RESOURCE_EXHAUSTED`, which names no field.

Authentication comes first among the remaining rows: the reference service authenticates each publish call before it inspects the request, so a caller whose credentials are missing or invalid gets `401` / `UNAUTHENTICATED` even when the same request is also malformed — an empty batch or a body that would fail schema validation is **not** answered `400` / `INVALID_ARGUMENT` or `422` in that case. Stated in full: an unauthenticated or unresolvable caller receives `401` / `UNAUTHENTICATED`; the only refusals that legitimately precede authentication are the two size bounds above — on REST the `413`, applied before the body is read, and on gRPC its transport-side analogue, the inbound message-size cap, which rejects an oversize message with `RESOURCE_EXHAUSTED` in the transport on receive, ahead of the RPC handler and of the authentication that runs with it on the already-deserialized request. Both are resource bounds rather than credential decisions, and neither tells a caller anything about its own credentials. Apart from them, every credential failure on the gRPC surface is `UNAUTHENTICATED`. A reimplementation should refuse in that order, so that a client can never learn a request-shape detail from a call it was not authenticated to make. The reference service resolves the token in its transport layer, ahead of the request body, so a REST request with an unresolvable token is answered `401` even when its body is not parseable JSON.

## Step 2: Implement the API in your language

Generate client/server stubs from the `.proto` with your toolchain of choice (for example `protoc`, `buf`, `tonic` for Rust, or the gRPC plugins for Go/Java), then implement the service methods. Honor the contract exactly:

- **Message shapes** — match the field names, types, and nesting in the `.proto` and `openapi.yaml` (notably the `google.protobuf.Struct` `message` payload and the optional `EventResource`).
- **Status codes and errors** — return the documented gRPC status codes / HTTP status codes for each error case (see *Status-code semantics* above).
- **Batch semantics** — batch publish fans out to the broker in parallel and returns one `PublishingResult` per input event, in request order; a partially failing batch still returns `200`/`OK` with per-event `success` flags.

You may implement the gRPC surface, the REST surface, or both — the conformance suite can be pointed at either or both.

## Step 3: Run the conformance suite

The `conformance_tests/` suite is the single runnable artifact that proves protocol compatibility. It is black-box: it talks to your running service over the wire and checks message shapes and status codes for the cases it covers — the success paths, the `422` / `INVALID_ARGUMENT` validation and empty-batch rejections, and the opt-in `401`. It does not exercise every row of the status-code table above; the `403`, `413`, `429`, and `500` cases are outside its scope today. Full run instructions, the complete list of what is tested, and configuration details live in `conformance_tests/README.md` — the summary below is a starting point.

Point the suite at your service with two environment variables and run it:

```bash
cd conformance_tests
uv sync

export CONFORMANCE_GRPC_ENDPOINT=localhost:50051
export CONFORMANCE_REST_ENDPOINT=http://localhost:8001

uv run run-conformance-tests
# or equivalently:
./run_tests.sh
```

To spin up the suite in a container instead, use the bundled docker-compose flow documented in `conformance_tests/README.md`.

### Scope of the suite

The bundled `conformance_tests/` suite exercises the publisher surface — `PublishEvent` and `BatchPublishEvents` (gRPC) and their REST equivalents (`POST /api/v1beta/events`, `POST /api/v1beta/events/batch`) — covering the success cases (single and batch publishes, result count matches input, `event_type` echoed) and the documented error cases (e.g. missing `occurred_at` → gRPC `INVALID_ARGUMENT` / HTTP 422, empty batch → `INVALID_ARGUMENT`). `conformance_tests/README.md` has the full case list.

The suite verifies the **wire protocol** — message shapes, status codes, and the error responses listed in `conformance_tests/README.md`; it does not cover every row of the status-code table above, and the `403`, `413`, `429`, and `500` cases are not exercised. It does **not** verify identity or authorization internals (how tokens are minted, signed, scoped, or mapped to permissions); auth-error (401) cases are opt-in via `CONFORMANCE_AUTH_REQUIRED=true`. If your service runs with auth enabled, supply a bearer token via `CONFORMANCE_AUTH_TOKEN` so the happy-path requests are authorized; our own runs disable auth to sidestep this, and you may do the same.

### Gating merges on conformance

To keep an alternative implementation compatible over time, run the conformance container in your own CI and fail the pipeline when any test fails. `conformance_tests/README.md` documents both a CI example and a Kubernetes (Helm) deployment of the suite for cluster-based gating.

## Configuration

The reference service reads its configuration from environment variables prefixed **`OMNI_EVENTS_`** (for example the gRPC bind address and REST port). A reimplementation is free to configure itself however it likes, but honoring the same prefix keeps drop-in deployment parity with the reference service.

## Customer documentation

The customer documentation for both notification services is published once, in the Event Consumer Service's folder, under `notifications/reference/event-consumer-service/docs/` (`../event-consumer-service/docs/` from here):

- **`docs.pdf`** — the complete, rendered manual, readable without building anything.
- **Markdown pages** (GitHub renders these inline):
  - `getting-started.md`
  - `publishing-events.md` — this service's publishing API.
  - `consuming-events.md`
  - `api-reference.md`
  - `permissions.md`
  - `index.md`
- **`images/`** — images referenced by the markdown pages.

These describe the notification APIs from a user's perspective and complement the machine-readable `.proto` and `openapi.yaml`. `consuming-events.md` covers the separate Event Consumer Service's consuming API, which is **not** part of the publisher contract — this guide targets the publisher surface only.

## Terminology

Use these terms consistently when reasoning about the contract:

- **Event Aggregation Service** — the service defined by this contract; the thing you are reimplementing. It accepts events and publishes them to the broker.
- **Event Consumer Service** — the downstream service that consumes events from the broker and delivers them to subscribers. Out of scope for a publisher reimplementation.
- **Single publish** — submit one event and receive its publishing result.
- **Batch publish** — submit multiple events published to the broker in parallel, with a per-event result for each.
