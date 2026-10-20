# Event Consumer Service — Conformance Tests

Blackbox API conformance tests for the **Event Consumer Service** (gRPC and REST).
They exercise the full API surface — happy paths and all documented error cases —
against a real running service. The goal is to give any reimplementor a
single runnable artefact that proves their implementation is protocol-compatible.

## Scope: what the suite does and does not verify

**In scope — the wire protocol.** The suite verifies gRPC and REST message
shapes, status codes, SSE event framing, and the documented error responses
for every RPC and endpoint. If your implementation returns a response whose
shape, status code, or error semantics differs from what the contract
specifies, the suite will catch it.

**Out of scope — identity and authorization.** The suite does not verify how
tokens are minted, signed, expired, scoped, or mapped to permissions. It
includes a small number of "request without Authorization header returns 401"
cases that can be enabled via `CONFORMANCE_AUTH_REQUIRED=true`, but these only
confirm that an auth-enabled service rejects anonymous calls — they do not
exercise expired tokens, audience/scope mismatches, or permission-layer
behavior (e.g. 403 on insufficient scope). Full auth-layer conformance is a
separate concern and is expected to be covered by implementation-specific
integration tests.

The `CONFORMANCE_AUTH_TOKEN` hook (and the Helm chart's `auth.existingSecretName`)
exists so reimplementors with auth-enabled services can still run the wire-protocol
suite by providing a valid bearer token on the happy-path requests. Our own CI
sidesteps this by running the service with auth disabled; reimplementors are
free to do the same.

## What is tested

| Scope | Endpoint | Cases |
|-------|----------|-------|
| gRPC | `CreateDurableQueue` | success (various filter types), unique IDs |
| gRPC | `DeleteDurableQueue` | success, NOT\_FOUND, double-delete |
| gRPC | `UpdateDurableQueue` | success, NOT\_FOUND, INVALID\_ARGUMENT (EQ + empty resource\_id) |
| gRPC | `ConsumeDurableEvents` | stream opens, NOT\_FOUND queue, empty queue\_id |
| gRPC | `ConsumeDurableEventsWithAcks` | unknown ack\_delivery\_tags tolerated, unknown nack\_delivery\_tags tolerated, same tag in both settle fields tolerated, NOT\_FOUND queue (settlement protocol only — see note below) |
| gRPC | `ConsumeNonDurableEvents` | stream opens with reconnect\_token, empty event\_type → INVALID\_ARGUMENT |
| REST | `GET /api/v1beta/health` | 200 + `{"status":"healthy"}` |
| REST | `POST /api/v1beta/queues/durable` | 200 + queue\_id, empty filters, unique IDs, 422 cases |
| REST | `DELETE /api/v1beta/queues/durable` | 200, 404, 422 (missing param), double-delete |
| REST | `PATCH /api/v1beta/queues/durable` | 200, 404, all 422 cases |
| REST | `GET /api/v1beta/events/stream` | 200 + SSE, `connected` event, reconnect token, 404, 400/422 (no filters or token), 422 |
| REST | `GET /api/v1beta/events/stream-durable` | 200 + SSE, `connected` event, 404, 422 |

`ConsumeDurableEventsWithAcks` coverage is limited to the settlement protocol:
that both settle fields are accepted on the wire, and that an unknown or
already-settled delivery tag in either field is skipped without terminating the
stream. The settlement *outcomes* — an acked message is not redelivered, a
nacked one is — are deliberately not asserted, because this suite has no way to
put a message into a durable queue. Events reach a durable queue only by being
published to the broker by the Event Aggregation Service, and the suite is a
black-box client of the Event Consumer Service alone (no AMQP client, since a
reimplementor need not use RabbitMQ). Those outcomes are covered against a live
broker by the service's own integration tests.

Auth-error cases (401) are gated behind `CONFORMANCE_AUTH_REQUIRED=true`.

## Prerequisites

- A running **Event Consumer Service** (gRPC on port 50052, REST on port 8000 by default)
- A connected **RabbitMQ** instance (required by the service for queue operations)

## Quick start — run locally

### Option A: against a locally running service

```bash
# 1. Install dependencies
cd conformance_tests
uv sync          # or: pip install -e .

# 2. Point at the service
export CONFORMANCE_GRPC_ENDPOINT=localhost:50052
export CONFORMANCE_REST_ENDPOINT=http://localhost:8000

# 3. Run
uv run run-conformance-tests
# or equivalently:
./run_tests.sh
```

### Option B: docker-compose (spins up everything)

```bash
# Build the conformance tests image
docker build -t event-consumer-conformance-tests:local .

# Run against the official image (replace with your version)
SERVICE_IMAGE=nvcr.io/nvidia/omniverse/event-consumer-service:1.9.9 \
  docker compose run --rm conformance-tests
```

Test reports land in `./reports/`.

## Configuration

All configuration is via environment variables.

| Variable | Default | Description |
|----------|---------|-------------|
| `CONFORMANCE_GRPC_ENDPOINT` | `localhost:50052` | gRPC `host:port` |
| `CONFORMANCE_REST_ENDPOINT` | `http://localhost:8000` | REST base URL |
| `CONFORMANCE_HEALTH_ENDPOINT` | `<REST_ENDPOINT>/api/v1beta/health` | Readiness probe URL |
| `CONFORMANCE_AUTH_TOKEN` | *(empty)* | Bearer token (omit for services with auth disabled) |
| `CONFORMANCE_AUTH_REQUIRED` | `false` | Set to `true` to run auth-error test cases |
| `CONFORMANCE_STREAM_TIMEOUT` | `10.0` | Seconds to wait for first SSE event |
| `CONFORMANCE_REQUEST_TIMEOUT` | `30.0` | HTTP request timeout (seconds) |

## Kubernetes deployment (Helm)

```bash
helm upgrade --install conformance-tests \
  oci://nvcr.io/nvidia/omniverse/event-consumer-conformance-tests \
  --version 1.9.9 \
  --set service.grpcEndpoint=event-consumer-service:50052 \
  --set service.restEndpoint=http://event-consumer-service:8000
```

To enable auth tests, first create a Secret in the release namespace containing
the bearer token, then point the chart at it:

```bash
kubectl create secret generic conformance-auth \
  --from-literal=token=<bearer-token>

helm upgrade --install conformance-tests \
  oci://nvcr.io/nvidia/omniverse/event-consumer-conformance-tests \
  --version 1.9.9 \
  --set auth.existingSecretName=conformance-auth \
  --set auth.required=true
```

## Interpreting results

Each test name describes exactly what it checks and what the service must return.
On failure, the assertion message explains:

- what was expected (e.g. `HTTP 404` or `gRPC NOT_FOUND`)
- what the service actually returned
- the relevant error details if available

Example failure output:
```
FAILED rest/test_delete_durable_queue.py::TestDeleteDurableQueue::test_delete_nonexistent_queue_returns_404
AssertionError: Expected HTTP 404 when deleting non-existent queue
'00000000-0000-0000-0000-000000000000', but got 200. Body: {"success": true}
```

## Adding the conformance tests as a CI gate

If you are building an alternative implementation and want to gate merges on passing
conformance, add a CI step that:

1. Starts your service (with auth disabled for simplicity).
2. Runs the conformance container, pointing it at your service.
3. Fails the pipeline if any test fails.

The step is the same in any CI system that can run a container: run the conformance
image on a network from which your service is reachable, pointed at its endpoints. The
image's default command waits for the service to become ready, then runs the tests, and
the container exits non-zero if any test fails.

```bash
docker run --rm \
  -e CONFORMANCE_GRPC_ENDPOINT=my-service:50052 \
  -e CONFORMANCE_REST_ENDPOINT=http://my-service:8000 \
  nvcr.io/nvidia/omniverse/event-consumer-conformance-tests:1.9.9
```
