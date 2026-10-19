# Discovery Service Conformance Tests

Blackbox conformance tests for the Discovery Service REST API.

## What is tested

| Endpoint | Tests |
|---|---|
| `GET /health` | 200 OK, body contains "healthy" |
| `GET /api/v1alpha/services` | 200 OK, valid JSON, schema-version=1, non-empty services, required fields, at least one endpoint per service |
| `GET /api/v1/services` | Same as v1alpha |

## Running locally

**Against a deployed service:**

```bash
cd conformance_tests
uv sync
CONFORMANCE_REST_ENDPOINT=http://my-discovery-service:8080 uv run run-conformance-tests -v
```

**With the bundled test NGINX (docker compose):**

```bash
cd conformance_tests
docker compose up --abort-on-container-exit --exit-code-from conformance-tests
```

The docker-compose starts a local NGINX serving test discovery JSON and then
runs the test suite against it.

## Environment variables

| Variable | Default | Description |
|---|---|---|
| `CONFORMANCE_REST_ENDPOINT` | `http://localhost:8080` | Base URL of the discovery service |
| `CONFORMANCE_HEALTH_ENDPOINT` | `http://localhost:8080/health` | Used by `wait_for_service.sh` |
| `CONFORMANCE_REQUEST_TIMEOUT` | `30.0` | Per-request timeout in seconds |
| `SKIP_OPTIONAL_TESTS` | `false` | Set to `true` to skip `@pytest.mark.optional` tests |

## Helm chart

The `helm/discovery-conformance-tests/` chart runs the conformance tests as a
Kubernetes Job against a service already deployed in the cluster.

```bash
helm install discovery-conformance-tests helm/discovery-conformance-tests \
  --set service.restEndpoint=http://discovery-service:8080 \
  --set service.healthEndpoint=http://discovery-service:8080/health
```
