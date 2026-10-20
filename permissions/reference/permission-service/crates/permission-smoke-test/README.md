# Permission Service Smoke Test

A smoke test binary for validating the permission-service Helm chart deployment. This test verifies core functionality by exercising the HTTP API endpoints.

## What It Tests

The smoke test performs the following verification steps:

1. **Health Check** - Verifies the service is running and healthy
2. **Add Policy** - Creates a test policy via `PUT /v1beta/policies/`
3. **Retrieve Policy** - Fetches the created policy via `GET /v1beta/policies/{id}`
4. **Authorization Check** - Verifies the policy grants access via `POST /v1beta/authorization/`
5. **Delete Policy** - Removes the test policy via `DELETE /v1beta/policies/{id}`
6. **Verify Deletion** - Confirms authorization is denied after policy removal

## Running the Binary

### Command Line Arguments

| Argument | Environment Variable | Required | Description |
|----------|---------------------|----------|-------------|
| `--service-url` | `SERVICE_URL` | Yes | Base URL of the permission service (e.g., `http://localhost:3000`) |
| `--wait-retries` | `WAIT_RETRIES` | No | Number of retries to wait for the service to be ready (default: `10`) |
| `--wait-interval` | `WAIT_INTERVAL` | No | Interval in seconds between health check retries (default: `5`) |
| `--openid-configuration` | `OPENID_CONFIGURATION` | No | Fallback OpenID configuration URI used by client credentials when `CLIENT_CREDENTIALS_OPENID_CONFIGURATION` is empty |

Outbound authentication is configured via one of the two flows below. They are mutually exclusive.

#### Client credentials flow

| Argument | Environment Variable | Required | Description |
|----------|---------------------|----------|-------------|
| `--enable-client-credentials` | `ENABLE_CLIENT_CREDENTIALS` | No | Enable OAuth2 client credentials authentication (default: `false`) |
| `--client-credentials-openid-configuration` | `CLIENT_CREDENTIALS_OPENID_CONFIGURATION` | When auth enabled* | OpenID discovery URI used for the client-credentials token exchange |
| `--client-id` | `CLIENT_ID` | When auth enabled | OAuth2 client ID |
| `--client-secret` | `CLIENT_SECRET` | When auth enabled** | OAuth2 client secret |
| `--client-secret-file` | `CLIENT_SECRET_FILE` | When auth enabled** | Path to file containing the client secret |
| `--client-scope` | `CLIENT_SCOPE` | When auth enabled | OAuth2 scope for the token request |
| `--extra-headers` | `CLIENT_CREDENTIALS_EXTRA_HEADERS` | No | JSON object of additional headers for the token request |
| `--extra-body-params` | `CLIENT_CREDENTIALS_EXTRA_BODY_PARAMS` | No | JSON object of additional body parameters for the token request |

\* Falls back to `OPENID_CONFIGURATION` when left empty.
\** Either `--client-secret` or `--client-secret-file` must be provided when authentication is enabled.

#### Access token file flow

| Argument | Environment Variable | Required | Description |
|----------|---------------------|----------|-------------|
| `--enable-access-token-file` | `ENABLE_ACCESS_TOKEN_FILE` | No | Read the bearer token from a file written by a sidecar (default: `false`) |
| `--access-token-file-path` | `ACCESS_TOKEN_FILE_PATH` | When enabled | Path to a JSON file with `access_token` and an RFC3339 `expires_on` |
| `--access-token-file-poll-interval` | `ACCESS_TOKEN_FILE_POLL_INTERVAL` | No | Polling interval (e.g. `15s`); unused by the short-lived smoke test but accepted for parity with the service |

### Examples

#### Without Authentication

```bash
# Using command line arguments
./permission-smoke-test --service-url http://localhost:3000

# Using environment variables
SERVICE_URL=http://localhost:3000 ./permission-smoke-test
```

#### With Client Credentials

```bash
# Using command line arguments
./permission-smoke-test \
  --service-url http://localhost:3000 \
  --enable-client-credentials \
  --client-id my-client-id \
  --client-secret my-client-secret \
  --client-scope "openid profile" \
  --client-credentials-openid-configuration https://auth.example.com/.well-known/openid-configuration

# Using environment variables
export SERVICE_URL=http://localhost:3000
export ENABLE_CLIENT_CREDENTIALS=true
export CLIENT_ID=my-client-id
export CLIENT_SECRET=my-client-secret
export CLIENT_SCOPE="openid profile"
export CLIENT_CREDENTIALS_OPENID_CONFIGURATION=https://auth.example.com/.well-known/openid-configuration
./permission-smoke-test

# Using a secret file
./permission-smoke-test \
  --service-url http://localhost:3000 \
  --enable-client-credentials \
  --client-id my-client-id \
  --client-secret-file /path/to/secret \
  --client-scope "openid profile" \
  --client-credentials-openid-configuration https://auth.example.com/.well-known/openid-configuration
```

#### With Access Token File

```bash
./permission-smoke-test \
  --service-url http://localhost:3000 \
  --enable-access-token-file \
  --access-token-file-path /etc/secrets/access-token/token
```

### Building from Source

```bash
cargo build --package permission-smoke-test --release
./target/release/permission-smoke-test --help
```

## Running with Helm in Kubernetes

The smoke test is integrated with the permission-service Helm chart as a test hook.

### Prerequisites

1. The permission-service Helm chart must be deployed
2. The smoke test container image must be available in your registry
3. Configure `smokeTest` values in your Helm values file

### Helm Values Configuration

```yaml
smokeTest:
  # Enable or disable the smoke test
  enabled: true

  image:
    # Container registry and image name
    repository: your-registry/permission-smoke-test
    # Image pull policy
    pullPolicy: IfNotPresent
    # Image tag (defaults to chart appVersion)
    tag: ""

  # Number of retries to wait for the service (total wait time = waitRetries * waitInterval)
  waitRetries: 10
  # Seconds between health check attempts
  waitInterval: 5

  # Resource limits and requests
  resources: {}
```

Outbound authentication is taken from the shared `serviceIdentity` block — the smoke
test reuses the same flow as the main deployment:

- `serviceIdentity.clientCredentials.enabled=true` enables the OAuth2 client-credentials
  flow. The smoke test mounts `serviceIdentity.clientCredentials.clientSecretRef` and uses
  `serviceIdentity.clientCredentials.openIdConfigurationUri` (defaulting to
  `openId.openIdConfigurationUri`) for token exchange.
- `serviceIdentity.accessTokenFile.enabled=true` enables the file-based flow. The smoke
  test runs the same `serviceIdentity.accessTokenFile.provisionerSidecar.container` and
  reads the token from `/etc/secrets/access-token/token` before running the tests.

### Running the Test

After deploying the Helm chart, run the smoke test using:

```bash
helm test <release-name>
```

For example:

```bash
# Run smoke test for a release named "permissions"
helm test permissions

# Run with increased timeout
helm test permissions --timeout 5m

# View test logs
helm test permissions --logs
```

### How It Works

When `helm test` is executed:

1. Kubernetes creates a Pod with the smoke test container
2. The smoke test polls the service health endpoint until it's ready (or times out based on `waitRetries` and `waitInterval`)
3. Once the service is ready, the smoke test runs the verification steps
4. The test connects to the permission service using the internal Kubernetes service URL
5. Test results are reported via the Pod's exit code (0 = success, non-zero = failure)
6. The Pod is automatically cleaned up after a successful test

The built-in retry logic prevents test failures when running `helm test` immediately after deployment, as it waits for the service to be fully ready before starting the tests.

### Viewing Test Results

```bash
# Check test pod status
kubectl get pods -l "app.kubernetes.io/instance=<release-name>" | grep smoke-test

# View test logs
kubectl logs <release-name>-permissions-smoke-test

# Describe pod for troubleshooting
kubectl describe pod <release-name>-permissions-smoke-test
```

## Building the Container Image

```bash
# Build the image
docker build -f docker/Dockerfile.smoke-test -t permission-smoke-test:latest .

# Push to registry
docker tag permission-smoke-test:latest your-registry/permission-smoke-test:latest
docker push your-registry/permission-smoke-test:latest
```

## Exit Codes

| Code | Description |
|------|-------------|
| 0 | All tests passed |
| 1 | One or more tests failed |


