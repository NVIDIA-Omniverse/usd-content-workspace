# Permission Service Conformance Test

Validates the permission-service REST and gRPC authorization APIs against expected behavior. Designed to run against any service implementation that follows the API contract defined in [AGENTS.md](AGENTS.md).

## Usage

```shell
cargo run --bin permission-service-conformance-test -- \
  --rest-url http://localhost:3000 \
  --grpc-url http://localhost:3010 \
  --principal conformance-test-principal \
  --action-name read \
  --action-service conformance-test \
  --resource-id conformance-test-resource \
  --resource-type ConformanceTestResource
```

The target service must already have authorization data configured for the given principal, action, and resource. See the `conformance-test-setup` binary in the `permission-service` crate for automated setup.

## Parameters

| CLI flag | Env var | Required | Description |
|---|---|---|---|
| `--rest-url` | `REST_URL` | For REST/all tests | HTTP base URL (e.g. `http://localhost:3000`) |
| `--grpc-url` | `GRPC_URL` | For gRPC/all tests | gRPC endpoint (e.g. `http://localhost:3010`) |
| `--test-suite` | `TEST_SUITE` | No | `all` (default), `rest`, or `grpc` |
| `--principal` | `PRINCIPAL` | Yes | Principal sub expected to be allowed |
| `--action-name` | `ACTION_NAME` | Yes | Action name expected to be allowed |
| `--action-service` | `ACTION_SERVICE` | Yes | Service name for the allowed action |
| `--resource-id` | `RESOURCE_ID` | Yes | Resource ID for the allowed action |
| `--resource-type` | `RESOURCE_TYPE` | Yes | Resource type for the allowed action |
| `--wait-retries` | `WAIT_RETRIES` | No | Health check retry count (default: 10) |
| `--wait-interval` | `WAIT_INTERVAL` | No | Seconds between retries (default: 5) |

### Authentication

When the target service has authentication enabled, provide credentials via one of:

- **Client credentials**: `--enable-client-credentials`, `--client-id`, `--client-secret` (or `--client-secret-file`), `--client-scope`, `--client-credentials-openid-configuration`
- **Access token file**: `--enable-access-token-file`, `--access-token-file-path`. The file must contain JSON with `access_token` and `expires_on` fields, where `expires_on` is RFC3339, for example `{"access_token":"...","expires_on":"2026-05-11T14:00:00Z"}`.

Run with `--help` for the full list.

## Test coverage

### REST

- **Health**: `GET /health` returns 200
- **Authorization allow**: `POST /v1beta/authorization/` with the configured principal/action/resource returns `decision: "allow"` and echoes `action`/`service` fields
- **Authorization deny**: same endpoint with a non-existent action (`conformance-test-nonexistent:deny-action`) returns `decision: "deny"`
- **Batch and**: `POST /v1beta/authorization/batch/` with `condition: "and"` combining an allowed and a denied action returns `summary.decision: "deny"` and short-circuits with `decision: "skip"` for actions after the first deny
- **Batch or**: same endpoint with `condition: "or"` returns `summary.decision: "allow"` and short-circuits with `decision: "skip"` for actions after the first allow
- **Batch none**: `condition: null` evaluates all actions individually with no summary and no skipped decisions

### gRPC

- **CheckPermission allow/deny**: `CheckPermission` RPC with allowed and denied payloads
- **CheckPermissionBatch and/or/unspecified**: `CheckPermissionBatch` RPC testing `CONDITION_AND`, `CONDITION_OR`, and `CONDITION_UNSPECIFIED` with the same short-circuit and summary rules as REST

### Deny tests

Deny tests use the hardcoded action `conformance-test-nonexistent:deny-action` which has no matching authorization rule in any correctly configured service. No CLI parameter is needed for deny scenarios.
