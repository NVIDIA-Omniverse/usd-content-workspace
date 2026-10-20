# Thumbnail Service Smoke Test

Standalone smoke test binary for validating the `thumbnail-service` HTTP API after deployment.

## What It Verifies

The smoke test executes a compact, end-to-end API flow:

1. Health check (`GET /health`)
2. Associate a synthetic thumbnail with the configured source file (`POST /v1alpha/associations`)
3. Retrieve the source file's thumbnails and verify that the association is returned (`GET /v1alpha/thumbnails`)
4. Remove the association (`DELETE /v1alpha/associations`)

## Usage

### Build

```bash
cargo build -p thumbnail-smoke-test
```

### Run

```bash
cargo run -p thumbnail-smoke-test -- \
  --service-url http://localhost:3000 \
  --source-file azure://smoke-tests/source-image.png \
  --auth-token test-token
```

### Environment Variables

`WAIT_INTERVAL` and `REQUEST_TIMEOUT` are measured in seconds.
When both are set, `AUTH_TOKEN_FILE` takes precedence over `AUTH_TOKEN`.
`AUTH_TOKEN_FILE` accepts either a raw bearer token string or JSON with
an `access_token` field (for sidecar-provisioned tokens).

| Argument | Environment Variable | Default |
|----------|----------------------|---------|
| `--service-url` | `SERVICE_URL` | _(required)_ |
| `--auth-token` | `AUTH_TOKEN` | _(empty)_ |
| `--auth-token-file` | `AUTH_TOKEN_FILE` | _(empty)_ |
| `--source-file` | `SOURCE_FILE` | _(required)_ |
| `--wait-retries` | `WAIT_RETRIES` | `10 retries` |
| `--wait-interval` | `WAIT_INTERVAL` | `5s` |
| `--request-timeout` | `REQUEST_TIMEOUT` | `15s` |

The process exits with code `0` when all checks pass, and `1` when any mandatory check fails.
