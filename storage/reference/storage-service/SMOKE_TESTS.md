# Storage Service Smoke Tests

Automated smoke tests for NVIDIA Omniverse Storage Service APIs (REST and gRPC).

## Overview

This project contains smoke tests that verify the functionality of the Storage Service by testing REST and gRPC API endpoints.

Tests run in Kubernetes via `helm test` or locally for development and debugging.

## Features

- ✅ **Kubernetes-native execution** - Tests run via `helm test` in the cluster
- ✅ **Local execution** - Run tests directly for faster iteration and debugging
- ✅ **30 comprehensive tests**: Storage (15 REST + 15 gRPC)
- ✅ **Configurable timeouts** and environment-based configuration
- ✅ **Pre-generated gRPC stubs** - No proto compilation needed at runtime

## Architecture

Tests are packaged in the Helm chart:
- Test files: `helm/files/tests/`
- Pre-generated gRPC stubs: `helm/files/generated/`
- ConfigMaps mount these files into the test pod
- Base image (`smoke-test-base`) provides Python runtime with dependencies

## Quick Start (Kubernetes)

**1. Install storage-service with tests enabled**

```bash
helm install storage-service ./helm \
  --set tests.enabled=true \
  --set tests.storageApiResourceBase="https://your-bucket.s3.amazonaws.com/test/"
```

**2. Run tests**

```bash
helm test storage-service
```

## Quick Start (Local Development)

For faster iteration and debugging, run tests directly on your machine.

**1. Create .env file**

```bash
cd tests

cat > .env << 'EOF'
# Required: Storage Service Endpoints (at least one)
SMOKE_TESTS_GRPC_ENDPOINT=https://storage.example.com:443
SMOKE_TESTS_REST_ENDPOINT=https://storage.example.com/api/v1

# Required: Resource base for test files
SMOKE_TESTS_RESOURCE_BASE=https://storage.example.com/path/

# Optional: Authentication
SMOKE_TESTS_AUTH_MICROSOFT_URL=https://login.microsoftonline.com/<tenant-id>/oauth2/v2.0/token
SMOKE_TESTS_AUTH_CREDENTIALS=<client-id>:<client-secret>
SMOKE_TESTS_AUTH_SCOPE=https://<your-scope>/.default

# Optional: Timeouts
SMOKE_TESTS_TIMEOUT=5

# Optional: Set to true to force insecure gRPC connection
SMOKE_TESTS_GRPC_SKIP_TLS=false
EOF
```

**2. Set up Python environment**

```bash
# Create and activate virtual environment
python3 -m venv .venv
source .venv/bin/activate

# Install dependencies
pip install grpcio protobuf pytest requests
```

**3. Run tests**

```bash
# Load environment variables
set -a && source .env && set +a

# Set PYTHONPATH to include pre-generated stubs and tests
export PYTHONPATH="../helm/files/generated:."

# Run all tests
python3 -m pytest smoke-tests/ -v

# Run specific test with debugging
python3 -m pytest smoke-tests/ -k test_stat_http -vv --log-cli-level=INFO

# Run only gRPC or REST tests
python3 -m pytest smoke-tests/ -m grpc -v
python3 -m pytest smoke-tests/ -m rest -v
```

## Usage

### Using Pytest Markers

Tests are organized with markers that allow you to run specific subsets:

| Marker | Description |
|--------|-------------|
| `rest` | REST API tests |
| `grpc` | gRPC API tests |

### Permissions

If the Storage Service has **permissions enabled**, you must configure a policy for the **resource base** used by the tests so that the test identity can read, write, and delete objects there. Without this policy, smoke tests will fail with permission-denied errors.

### Testing Multiple Storage Backends

You can test against multiple storage backends (e.g., Azure Blob, S3) in a single test run by providing a comma-separated list of resource bases:

```bash
# Single backend
SMOKE_TESTS_RESOURCE_BASE="https://blob.azure.com/container/path/"

# Multiple backends
SMOKE_TESTS_RESOURCE_BASE="https://blob.azure.com/bucket1/,https://s3.aws.com/bucket2/"
```

### Helm Test Configuration

Configure tests via the chart `values.yaml`:

- **Endpoints**: Optional. If set, smoke tests use `tests.grpcStorageEndpoint` and `tests.httpStorageEndpoint`. If not set, default is ingress URLs (from `httpProxy.fqdn`) when ingress is enabled, or in-cluster URLs when disabled.

**Smoke tests through ingress:** To run tests via ingress you need all of the following. When they are in place, the tests use the ingress endpoints and auth and will pass.

1. **REST and gRPC endpoints** – The chart sets these from `httpProxy.fqdn` when ingress is enabled: REST = `https://<host>.<domain>` (or `http://` if TLS is off), gRPC = `https://grpc.<host>.<domain>` (or `http://grpc.<host>.<domain>`). Example: `https://storage.example.com` and `https://grpc.storage.example.com`.
2. **Microsoft auth** – Set `tests.auth` with `microsoftUrl` (OIDC discovery URL), `credentials` (client_id:client_secret), and `scope` so the test pod can obtain a Bearer token.
3. **DNS** – The hostnames above must resolve to your ingress (e.g. the Envoy LoadBalancer IP). Ensure DNS or internal resolution is configured so the test pod can reach the ingress; otherwise you will see "Failed to resolve hostname" and tests will fail.

```yaml
tests:
  enabled: true
  runGrpcTests: true
  runRestTests: true
  storageApiResourceBase: "https://your-bucket.s3.amazonaws.com/test/"

  # Optional: override endpoints; else default is ingress URLs (when ingress on) or in-cluster
  grpcStorageEndpoint: ""
  httpStorageEndpoint: ""

  # Optional: Timeouts and TLS
  timeout: "30"
  grpcSkipTls: ""
  restSkipTls: ""

  # Optional: Authentication (required when using ingress or when service has auth enabled)
  auth:
    microsoftUrl:
      value: "https://login.microsoftonline.com/.../v2.0/.well-known/openid-configuration"
    credentials:
      valueFrom:
        secretKeyRef:
          name: "oauth-secrets"
          key: "credentials"
    scope:
      value: "https://.../.default"

smokeTest:
  image:
    repository: nvcr.io/nvidia/omniverse/smoke-test-base
    tag: "1.0.1"
    pullPolicy: IfNotPresent
    pullSecrets: []
```

## Test Coverage

### Storage REST API Tests (15)

| Test | Operation | Endpoint |
|------|-----------|----------|
| test_list_services_http | GET | /v1alpha/capabilities/services |
| test_list_top_level_addresses_http | GET | /v1alpha/capabilities/top-level-addresses |
| test_list_routes_http | GET | /v1alpha/capabilities/routes |
| test_stat_http | HEAD | /v1alpha/fileobject/by-address/{address} |
| test_read_from_address_http | GET | /v1alpha/fileobject/by-address/{address} |
| test_write_http | PUT | /v1alpha/fileobject/by-address/{address} |
| test_delete_http | DELETE | /v1alpha/fileobject/by-address/{address} |
| test_read_by_identity_http | GET | /v1alpha/fileobject/by-identity/{identity} |
| test_copy_http | POST | /v1alpha/fileobject/by-identity/{identity}/copy |
| test_enumerate_versions_http | GET | /v1alpha/versioning/{address}/versions |
| test_filefolder_list_http | GET | /v1alpha/filefolder/list/{path} |
| test_filefolder_liststat_http | GET | /v1alpha/filefolder/liststat/{path} |
| test_update_metadata_http | PUT | /v1alpha/metadata/{address}/{key} |
| test_get_metadata_http | POST | /v1alpha/metadata/{address} |
| test_delete_metadata_http | DELETE | /v1alpha/metadata/{address}/{key} |

### Storage gRPC API Tests (15)

| Test | RPC Method | Service |
|------|-----------|---------|
| test_list_services_grpc | ListServices | CapabilitiesService |
| test_list_top_level_addresses_grpc | ListTopLevelAddresses | CapabilitiesService |
| test_list_routes_grpc | ListRoutes | CapabilitiesService |
| test_write_grpc | Write | FileObjectService |
| test_stat_grpc | Stat | FileObjectService |
| test_readfromaddress_grpc | ReadFromAddress | FileObjectService |
| test_delete_grpc | Delete | FileObjectService |
| test_read_by_identity_grpc | Read | FileObjectService |
| test_copy_grpc | Copy | FileObjectService |
| test_enumerate_versions_grpc | EnumerateVersions | VersioningService |
| test_list_grpc | List | FileFolderService |
| test_liststat_grpc | ListStat | FileFolderService |
| test_update_metadata_grpc | UpdateMetadata | MetadataService |
| test_get_metadata_grpc | GetMetadata | MetadataService |
| test_delete_metadata_grpc | DeleteMetadata | MetadataService |

## Environment Variables Reference

### Required

| Variable | Description | Example |
|----------|-------------|---------|
| `SMOKE_TESTS_RESOURCE_BASE` | Base path(s) for test resources (comma-separated for multiple). When permissions are enabled, ensure a policy allows the test identity to access these paths. | `https://storage.example.com/test/` |

### Required - Service Endpoints (at least one)

| Variable | Description | Example |
|----------|-------------|---------|
| `SMOKE_TESTS_GRPC_ENDPOINT` | Storage gRPC endpoint | `https://storage.example.com:443` |
| `SMOKE_TESTS_REST_ENDPOINT` | Storage REST endpoint | `https://storage.example.com/api/v1` |

### Optional - Authentication

| Variable | Description | Example |
|----------|-------------|---------|
| `SMOKE_TESTS_AUTH_MICROSOFT_URL` | OAuth token endpoint URL | `https://login.microsoftonline.com/.../oauth2/v2.0/token` |
| `SMOKE_TESTS_AUTH_CREDENTIALS` | Client credentials | `<client-id>:<client-secret>` |
| `SMOKE_TESTS_AUTH_SCOPE` | OAuth scope | `https://.../.default` |

### Optional - Configuration

| Variable | Description | Default |
|----------|-------------|---------|
| `SMOKE_TESTS_TIMEOUT` | Request timeout in seconds | `5` |
| `SMOKE_TESTS_GRPC_SKIP_TLS` | Skip TLS for gRPC (true/false) | `false` |
| `PYTEST_ADDOPTS` | Additional pytest CLI options ([docs](https://docs.pytest.org/en/stable/example/simple.html#how-to-change-command-line-options-defaults)) | `--log-cli-level=INFO --tb=no` |

## Updating gRPC Stubs

If proto files change, regenerate the stubs:

```bash
cd helm/files

# Install grpcio-tools if needed
pip install grpcio-tools

# Get latest proto files from storage-protos repo
# ... copy proto files to a temp directory ...

# Generate stubs
python -m grpc_tools.protoc \
  --proto_path=<proto-dir> \
  --python_out=generated \
  --pyi_out=generated \
  --grpc_python_out=generated \
  <proto-dir>/nvidia/omniverse/storage/*/v1alpha/*.proto

# Create __init__.py files
find generated -type d -exec touch {}/__init__.py \;
```
