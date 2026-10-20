# Event Aggregation Service — Conformance Tests

Blackbox API conformance tests for the **Event Aggregation Service** (gRPC and REST).
They exercise the full API surface — happy paths and all documented error cases —
against a real running service. The goal is to give any reimplementor a
single runnable artefact that proves their implementation is protocol-compatible.

## What is tested

| Scope | Endpoint | Cases |
|-------|----------|-------|
| gRPC | `PublishEvent` | success (with/without resource), echoes event_type, missing occurred_at → INVALID_ARGUMENT, message over 65536 JSON bytes / event_type over 256 chars / resource_id over 2048 chars → INVALID_ARGUMENT naming the field, message and event_type exactly at the limit → success |
| gRPC | `BatchPublishEvents` | single event, multiple events, result count matches input, empty list / over 1000 events / an oversize message → INVALID_ARGUMENT, exactly 1000 events → success |
| REST | `GET /api/v1beta/health` | 200 + `{"status":"healthy"}` |
| REST | `POST /api/v1beta/events` | 200 + success=true, no resource, timestamp variants, echo, all 422 cases (incl. message over 65536 JSON bytes, event_type over 256 chars, resource_id over 2048 chars, occurred_at over 64 chars), message exactly at the limit → 200 |
| REST | `POST /api/v1beta/events/batch` | 200, result count, all success, 400 (empty list), 422 cases (incl. over 1000 events and an oversize message) |

Auth-error cases (401) are gated behind `CONFORMANCE_AUTH_REQUIRED=true`.

## Prerequisites

- A running **Event Aggregation Service** (gRPC on port 50051, REST on port 8001 as `uv run start` and the bundled `docker-compose.yml` run it)
- A connected **RabbitMQ** instance (required by the service for event publishing)

## Quick start — run locally

### Option A: against a locally running service

```bash
# 1. Install dependencies
cd conformance_tests
uv sync          # or: pip install -e .

# 2. Point at the service
export CONFORMANCE_GRPC_ENDPOINT=localhost:50051
export CONFORMANCE_REST_ENDPOINT=http://localhost:8001

# 3. Run
uv run run-conformance-tests
# or equivalently:
./run_tests.sh
```

### Option B: docker-compose (spins up everything)

```bash
# Build the conformance tests image
docker build -t event-aggregation-conformance-tests:local .

# Run against the official image (replace with your version)
SERVICE_IMAGE=nvcr.io/nvidia/omniverse/event-aggregation-service:1.0.0 \
  docker compose run --rm conformance-tests
```

Test reports land in `./reports/`.

## Configuration

All configuration is via environment variables.

| Variable | Default | Description |
|----------|---------|-------------|
| `CONFORMANCE_GRPC_ENDPOINT` | `localhost:50051` | gRPC `host:port` |
| `CONFORMANCE_REST_ENDPOINT` | `http://localhost:8001` | REST base URL |
| `CONFORMANCE_HEALTH_ENDPOINT` | `<REST_ENDPOINT>/api/v1beta/health` | Readiness probe URL |
| `CONFORMANCE_AUTH_TOKEN` | *(empty)* | Bearer token (omit for services with auth disabled) |
| `CONFORMANCE_AUTH_REQUIRED` | `false` | Set to `true` to run auth-error test cases |
| `CONFORMANCE_REQUEST_TIMEOUT` | `30.0` | HTTP request timeout (seconds) |
| `SKIP_OPTIONAL_TESTS` | *(not set)* | Set to `true` to skip `@pytest.mark.optional` tests |

## Kubernetes deployment (Helm)

```bash
helm upgrade --install conformance-tests \
  oci://nvcr.io/nvidia/omniverse/event-aggregation-conformance-tests \
  --version 1.0.0 \
  --set service.grpcEndpoint=event-aggregation-service:50051 \
  --set service.restEndpoint=http://event-aggregation-service:8000 \
  --set service.healthEndpoint=http://event-aggregation-service:8000/api/v1beta/health
```

The REST and health port is the service chart's `rest.port` (default `8000`).

To enable auth tests:
```bash
  --set auth.token=<bearer-token> \
  --set auth.required=true
```

## Interpreting results

Each test name describes exactly what it checks and what the service must return.
On failure, the assertion message explains:

- what was expected (e.g. `HTTP 422` or `gRPC INVALID_ARGUMENT`)
- what the service actually returned
- the relevant error details if available

Example failure output:
```
FAILED rest/test_publish_event.py::TestPublishEvent::test_publish_missing_occurred_at_returns_422
AssertionError: Expected HTTP 422 for missing occurred_at,
but got 200. Body: {"result": {"success": true}}
```
