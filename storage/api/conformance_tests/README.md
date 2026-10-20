# Omniverse Storage API conformance tests 

The conformance test suite verifies that a Storage API implementation behaves in a way that generic client software 
can reliably depend on.

The tests are written in the Gherkin specification language and executed via `pytest-bdd`. They focus on
observable behavior and return values rather than internal implementation details.

This is **not** a comprehensive quality or performance benchmark for a storage service. Instead, it is
intended to ensure that different Storage API implementations expose a consistent contract so that clients
can interact with them in a uniform way.

A complete runtime implementation – fixtures, step definitions and test data generators – is provided so the
suite can be run as a blackbox against any Storage API service that exposes the expected endpoints.

## Installation

### Prerequisites

- **Python** 3.10, 3.11, or 3.12
- **Poetry** package manager

```bash
# Run  these commands within the conformance tests subdirectory
cd conformance_tests

# Create virtual environment for poetry
python -m venv .poetry_venv

# Install poetry 
.poetry_venv/bin/pip install poetry
```

### Build from source

Before installing from a fresh Git checkout, generate the ignored `src/nvidia` protobuf files
for both Python packages. From the repository root, follow the “Generate Python protobuf stubs
without Packman” section of the root `README.md`, or, after running the Packman bootstrap, use
`make sync-conformance-test-nvidia sync-filesystem-example-nvidia`. Regenerate the stubs after
changing a `.proto` file.

```bash
# Install dependencies
.poetry_venv/bin/poetry install 

# Activate the poetry-created virtual environment
source .venv/bin/activate
```

## Usage

The test suite is exposed as a Python package with a small command line entrypoint. Once installed in an
environment, the tests can be invoked against any Storage API implementation that is reachable from the
test runner.

### 1. Start a Storage API implementation

Start the Storage API service you want to validate and ensure it is reachable from the machine running the
test suite. For the reference filesystem service refer to its documentation.

```bash
# Start the service with default ports
local-filesystem-service filesystem --static-dir /tmp/storage
```

This starts a Storage API implementation with the following default endpoints:

- REST endpoint: `http://localhost:8011`
- gRPC endpoint: `localhost:50051`
- Resource address base: `file-storage://fileservice`

These defaults match the conformance test suite defaults, so no additional configuration is required for a
basic local run.

### 2. Running the tests via the entrypoint

After installing the `omniverse-storageapi-conformance-tests` package into an environment (for example via
`poetry install` in the `conformance_tests` directory), the tests can be run with:

```bash
# After successfully running `poetry install` above, activate the environment poetry created:
source .venv/bin/activate

# In this environment, the run hook has been installed for the test suite:
run-conformance-tests
```

This command:

- Uses `pytest` under the hood
- Loads the default test data generator plugin
  (`conformance_tests.example_fixtures.storageapi_testdata_generator`)
- Executes the full Gherkin-based conformance suite against the configured Storage API endpoints

Any extra command line arguments are forwarded directly to `pytest`. For example, to select a subset of tests
or increase verbosity:

```bash
run-conformance-tests -k "stat" -vv
```

Native metadata scenarios can be selected the same way:

```bash
run-conformance-tests -k "native_metadata"
```

## Configuration

The conformance tests are intentionally minimal in terms of configuration: they assume a running Storage API
implementation and use a small set of environment variables to control the endpoints, the resource address
base and some test behavior.

### Core Storage API configuration

These environment variables are used by the default `storageapi_testdata_generator` fixture
(`conformance_tests.example_fixtures.storageapi_testdata_generator`). This test data generator, a subclass
of the conformance_tests.storage_testdata_generator.AbstractTestDataGenerator base class, uses only the 
Storage API itself to create test modify test data. This is limited for a few test cases like permission modification,
so to extend test coverage a bespoke TestDataGenerator implementation can be useful for certain storage backends.

- **`TEST_STORAGE_API_REST_ENDPOINT`** (default: `http://localhost:8011`)
  - Base URL of the Storage API REST endpoint used by the tests.
  - Must point to the root where the versioned APIs are exposed, e.g. `http://host:8011`.

- **`TEST_STORAGE_API_GRPC_ENDPOINT`** (default: `localhost:50051`)
  - Host and port of the gRPC endpoint implementing the Storage API.
  - Used to create an insecure gRPC channel for the `FileObjectService`, `FileFolderService` and
    `CapabilitiesService` stubs.

- **`TEST_STORAGE_API_RESOURCE_BASE`** (default: `file-storage://fileservice`)
  - Base resource address prefix used when generating test namespaces and resource addresses.
  - Must be a valid Storage API resource address prefix that is recognized by the service under test.

- **`STORAGEAPI_TEST_HTTP_TIMEOUT`** (default: `60` seconds)
  - Global timeout used by the REST client helper for all HTTP calls made by the tests.
  - Increase this value if your Storage API implementation has higher latency or performs expensive
    backend operations during the tests.

Some environment variables control corner cases and error conditions used by the test data generators:

- **`TEST_INVALID_RESOURCE_ADDRESS`** (default: `c:d:e:\0`)
  - Value returned by the test data generator when an intentionally invalid resource address is required.
  - You can override this if your implementation uses different validation rules for invalid addresses.

- **`TEST_INVALID_RESOURCE_IDENTITY`** (default: `c:d:e:\0`)
  - Value returned when an intentionally invalid resource identity is required.

These values are only used in negative test cases and should not correspond to valid addresses or identities
in your Storage API implementation.

### Native metadata conformance

The FileObject native metadata tests cover the optional `native_metadata` fields returned by `Write()`, `Stat()`,
`Read()`, `ReadFromAddress()`, `Enumerate()`, and `ListStat()` when native metadata is explicitly requested. Native
metadata is scoped to the written object version. Support probing is address-scoped: clients must query support for the
exact resource address they intend to write, after any service routing or top-level address selection has been applied.

Clients must not infer support from an empty `native_metadata` field. Empty or absent native metadata can mean that the
service supports native metadata but no metadata was provided, that the object version has no stored native metadata, or
that the route does not support native metadata. Use `GetNativeMetadataSupport` for gRPC or the REST native metadata
support endpoint for the exact resource address instead.

Services may return backend-generated native metadata that the client did not write, such as an object ETag. The
conformance checks assert that client-supplied entries survive on the written version; they do not require the returned
native metadata map to contain only those entries.

The conformance guard classifies support queries as:

- `supported`: the support endpoint is present and returns `supports_native_metadata=true`.
- `unsupported_present`: the support endpoint is present and returns `supports_native_metadata=false`.
- `absent_endpoint`: the support endpoint itself is not implemented.
- `error`: malformed address, permission denial, invalid response, or another unexpected failure.

Supported-service scenarios require `supported`. Unsupported-service contract scenarios require `unsupported_present`.
Older services with an absent endpoint are treated as "do not send native metadata" by client helpers, but the
unsupported write contract is only asserted when the service explicitly reports `supports_native_metadata=false`.

For an unsupported route, ordinary writes without native metadata still follow the existing FileObject contract and all
native metadata returned by `Write()`, `Stat()`, `Read()`, `ReadFromAddress()`, and `Enumerate()` must be empty or absent.
Explicit empty native metadata is equivalent to omission and is not a support signal. When the support endpoint exists
and reports `supports_native_metadata=false`, valid non-empty native metadata writes must be rejected before creating or
mutating an object: gRPC returns `UNIMPLEMENTED`, and REST returns `501`.
Malformed addresses and write-authorization failures keep their normal precedence, returning `INVALID_ARGUMENT`/`400`
or `PERMISSION_DENIED`/`403` rather than a native metadata support error.

Older services with no support endpoint may ignore unknown native metadata input and complete ordinary writes without
storing it.

When using the reference filesystem service, the native metadata selector can be run in both modes from the repository
root:

```bash
# Default reference service mode: native metadata support enabled
./run_conformance_test.sh fixtures.filesystem_service "native_metadata"

# Explicit unsupported mode
FILESERVICE_NATIVE_METADATA_MODE=disabled ./run_conformance_test.sh fixtures.filesystem_service "native_metadata"
```

`FILESERVICE_NATIVE_METADATA_MODE=enabled` is the default. The disabled run is useful for validating clients and services
that route some addresses to backends without native metadata storage.

### OpenAPI verification

The test suite includes optional checks that validate the OpenAPI schemas exposed by the Storage API service
against the reference schemas shipped with this repository.

- **`OASDIFF_EXECUTABLE`** (required for OpenAPI tests)
  - Path to the `oasdiff` binary used to compare schemas.
  - Must point to an executable on the system running the tests.

- **`TEST_EXACT_OPENAPI_MATCH`** (default: `false`)
  - When set to `true`, enables strict OpenAPI schema comparison tests.
  - If this variable is not set to `"true"` (case-insensitive), the exact match tests are skipped.

### S3/Boto3-based test data generator
The conformance test suite by default uses the Storage API itself to create and delete test data. This somewhat 
restricts the usefulness of the tests as you might have "symmetry" errors not caught when both test data creation
and the test function use the same code path. Also, some tests are not enabled as the Storage API for example cannot
modify permissions, therefore no tests for missing permissions can be created.

As an alternative it is possible to subclass AbstractTestDataGenerator and build a test data generator that natively
handles test data on the storage backend, giving better test coverage.

As an example, an alternative test data generator is provided in
`conformance_tests.example_fixtures.boto3_testdata_generator`. It uses `boto3` to store the test data in an
S3-compatible object store. To use it, set the pytest plugin accordingly (via `PYTEST_PLUGINS).

This generator uses the usual AWS configuration environment variables plus a few additional ones:

- **`AWS_ACCESS_KEY_ID`**, **`AWS_SECRET_ACCESS_KEY`**, **`AWS_REGION`** (default: `us-east-1`)
  - Standard AWS credentials and region configuration used by `boto3.Session()`.

- **`TEST_STORAGE_API_BOTO3_CONNECT_TIMEOUT`** (default: `5.0` seconds)
  - Connection timeout used when constructing the S3 client.

- **`TEST_STORAGE_API_BOTO3_MAX_POOL_CONNECTIONS`** (default: `20`)
  - Maximum number of pooled HTTP connections for the S3 client.

- **`TEST_STORAGE_API_BOTO3_BUCKET_NAME`** (default: `sapiv`)
  - Name of the bucket where the test objects will be created.

- **`CREATE_FOLDER`** (default: `false`)
  - When set to `true`, the S3 generator will create zero-byte objects to explicitly represent folders.
  - When `false`, folders are inferred from object keys only.

In addition, when the configured S3 endpoint looks like a local or MinIO-style deployment, the generator
automatically sets:

- **`BOTO_EXPERIMENTAL__NO_EMPTY_CONTINUE`** to `true`
  - This is an internal boto3 flag used to work around known issues with certain S3-compatible implementations.

### Test execution control

The following environment variables affect how the tests themselves are executed, rather than what they test:

- **`PYTEST_PLUGINS`**
  - When unset, the `run-conformance-tests` entrypoint defaults to
    `conformance_tests.example_fixtures.storageapi_testdata_generator`.
  - You can set it to another plugin (for example the boto3 generator, or your own implementation of the AbstractTestDataGenerator class) to change how test data is created.

## Reporting and logs

The conformance tests write their standard output, logging and `pytest` reports to the current working
directory.

## Summary

- Start a Storage API implementation that exposes the REST and/or gRPC endpoints described in the Storage API
  specification.
- Build, install and configure the `omniverse-storageapi-conformance-tests` package.
- Point the tests at your implementation via the environment variables described above.
- Run `run-conformance-tests` and inspect the results to verify conformance of your implementation.
