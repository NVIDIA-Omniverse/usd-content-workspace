<!-- SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# AI Agent Guide for Reimplementing the NVIDIA Omniverse Event Consumer Service

This document guides AI coding agents (Cursor, GitHub Copilot, Codex, and similar) and human engineers who want to build a protocol-compatible reimplementation of the NVIDIA Omniverse **Event Consumer Service** from its published API contract.

The goal is concrete: read the API specifications, implement the gRPC and/or REST surface in the language of your choice, and prove compatibility by running the bundled conformance suite against your implementation until it passes.

## Project overview

The Event Consumer Service lets clients subscribe to and consume events that the upstream **Event Aggregation Service** publishes to a message broker. It exposes two interchangeable API surfaces over the same operations:

- **gRPC** (primary) — the `EventConsumerService` contract.
- **REST/HTTP** (optional) — the same operations with Server-Sent Events (SSE) for streaming.

Both surfaces support consuming events from two queue types and managing durable queues:

- **Durable queue** — explicitly created, persists across client disconnects, and buffers messages while the queue exists.
- **Non-durable queue** — ephemeral per-connection, deleted on disconnect, with reconnect-token support for short-window reconnection.

This folder, `notifications/reference/event-consumer-service` in the `usd-content-workspace` repository, holds the reference implementation together with the published API contract. All paths below are relative to this folder. The contract consists of:

- **API specifications** — the gRPC `.proto`, published once for all notification services under `notifications/api` (`../../api` from here), and the REST OpenAPI `openapi.yaml` in this folder.
- **A conformance suite** — a runnable, black-box test artifact that validates any implementation against the wire protocol.
- **Customer documentation** — the rendered manual and markdown pages for both notification services.

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
| `../../api/protos/nvidia/omniverse/notifications/consumer/v1beta/event_consumer.proto` | The gRPC `EventConsumerService` contract: consume durable/non-durable events, durable-queue CRUD, and dead-letter triage. The authoritative interface definition. |
| `../../api/protos/buf.yaml` | The buf module configuration for the proto tree. |
| `openapi.yaml` | The REST specification: paths, request/response schemas, status codes, and SSE framing. Generated from the service — treat it as read-only reference; do not hand-edit it. |
| `conformance_tests/` | The black-box conformance suite. See `conformance_tests/README.md` for how to run it. |
| `docs/` | The consumer-facing documentation (see *Customer documentation* below). |

## Step 1: Read the API contract

Start with `../../api/protos/nvidia/omniverse/notifications/consumer/v1beta/event_consumer.proto` — it is the primary surface. The gRPC contract covers:

- **`ConsumeDurableEvents`** — stream events from an existing durable queue. **Deprecated**: the server auto-acknowledges messages after a short delay, which leaves a window for message loss on disconnect. Use `ConsumeDurableEventsWithAcks` instead; this RPC will be removed in a future release.
- **`ConsumeDurableEventsWithAcks`** — the recommended durable-consume RPC: a bidirectional stream that delivers events with per-message delivery tags and requires the client to acknowledge them explicitly, closing the auto-ack message-loss window. Unacknowledged messages are redelivered on reconnect a bounded number of times, then moved to a dead-letter queue.
- **`ConsumeNonDurableEvents`** — stream events over an ephemeral queue, with reconnect-token support.
- **`CreateDurableQueue`** — create a durable queue (optionally with a client-specified queue id).
- **`UpdateDurableQueue`** — change a durable queue's filters.
- **`DeleteDurableQueue`** — delete a durable queue.
- **`ManageDeadLetteredEvents`** — a bidirectional stream for an operator to inspect and triage dead-lettered events across the deployment. The server streams each dead-lettered message and the client replies with a per-message action (replay, discard, or leave); the server streams back an outcome for each action.

If you are also implementing the REST surface, read `openapi.yaml`. It maps the same operations onto HTTP, using SSE for the streaming endpoints. The REST endpoints the conformance suite exercises include:

- `GET /api/v1beta/health`
- `POST /api/v1beta/queues/durable`
- `PATCH /api/v1beta/queues/durable`
- `DELETE /api/v1beta/queues/durable`
- `GET /api/v1beta/events/stream` (non-durable, SSE)
- `GET /api/v1beta/events/stream-durable` (durable, SSE)

## Step 2: Implement the API in your language

Generate client/server stubs from the `.proto` with your toolchain of choice (for example `protoc`, `buf`, `tonic` for Rust, or the gRPC plugins for Go/Java), then implement the service methods. Honor the contract exactly:

- **Message shapes** — match the field names, types, and nesting in the `.proto` and `openapi.yaml`.
- **Status codes and errors** — return the documented gRPC status codes / HTTP status codes for each error case (for example `NOT_FOUND` / `404` for a missing queue, `INVALID_ARGUMENT` / `422` for a bad request).
- **Streaming semantics** — durable and non-durable consume are long-lived streams; the REST surface frames events as SSE, including the initial `connected` event and reconnect-token handling.

You may implement the gRPC surface, the REST surface, or both — the conformance suite can be pointed at either or both.

## Step 3: Run the conformance suite

The `conformance_tests/` suite is the single runnable artifact that proves protocol compatibility. It is black-box: it talks to your running service over the wire and checks message shapes, status codes, SSE framing, and every documented error response. Full run instructions, the complete list of what is tested, and configuration details live in `conformance_tests/README.md` — the summary below is a starting point.

Point the suite at your service with two environment variables and run it:

```bash
cd conformance_tests
uv sync

export CONFORMANCE_GRPC_ENDPOINT=localhost:50052
export CONFORMANCE_REST_ENDPOINT=http://localhost:8000

uv run run-conformance-tests
# or equivalently:
./run_tests.sh
```

To spin up the suite in a container instead, use the bundled docker-compose flow documented in `conformance_tests/README.md`.

### Scope of the suite

The bundled `conformance_tests/` suite currently exercises only the five core RPCs (`ConsumeDurableEvents`, `ConsumeNonDurableEvents`, `CreateDurableQueue`, `UpdateDurableQueue`, `DeleteDurableQueue`) and their REST equivalents. It does **not** yet cover `ConsumeDurableEventsWithAcks` or `ManageDeadLetteredEvents`, so a green conformance run does not by itself prove those two RPCs are compatible — validate them with your own tests against the `.proto` contract.

The suite verifies the **wire protocol** — message shapes, status codes, SSE event framing, and documented error responses for every RPC and endpoint it covers. It does **not** verify identity or authorization internals (how tokens are minted, signed, scoped, or mapped to permissions); it offers only an opt-in "anonymous request is rejected" check via `CONFORMANCE_AUTH_REQUIRED=true`. Full auth-layer conformance is a separate concern for your own integration tests. If your service runs with auth enabled, supply a bearer token via `CONFORMANCE_AUTH_TOKEN` so the happy-path requests are authorized; our own runs disable auth to sidestep this, and you may do the same.

### Gating merges on conformance

To keep an alternative implementation compatible over time, run the conformance container in your own CI and fail the pipeline when any test fails. `conformance_tests/README.md` documents both a CI example and a Kubernetes (Helm) deployment of the suite for cluster-based gating.

## Customer documentation

The customer documentation for both notification services ships in this folder under `docs/`:

- **`docs/docs.pdf`** — the complete, rendered manual, readable without building anything.
- **Markdown pages** (GitHub renders these inline):
  - `docs/getting-started.md`
  - `docs/consuming-events.md`
  - `docs/publishing-events.md`
  - `docs/api-reference.md`
  - `docs/permissions.md`
  - `docs/index.md`

These describe the notification APIs from a user's perspective and complement the machine-readable `.proto` and `openapi.yaml`. `docs/publishing-events.md` covers the separate Event Aggregation Service's publishing API, which is **not** part of the consumer contract — this guide targets the consumer surface only.

## Terminology

Use these terms consistently when reasoning about the contract:

- **Event Consumer Service** — the service defined by this contract; the thing you are reimplementing.
- **Event Aggregation Service** — the upstream service that publishes events to the broker. Out of scope for a consumer reimplementation.
- **Durable queue** — explicitly created, persists across disconnects, buffers messages while it exists.
- **Non-durable queue** — ephemeral per-connection, deleted on disconnect, with reconnect-token support.
