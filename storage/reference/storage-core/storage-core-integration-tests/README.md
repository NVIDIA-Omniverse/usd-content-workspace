# Storage Core Integration Tests

This crate contains integration tests for the `storage-core` library. The tests are organized in their own crate for several important architectural reasons:

## Why a Separate Crate?

### Feature Flag Control
Integration tests need to control the feature flags of the `storage-core` dependency. By having tests in their own crate, we can:
- Enable/disable specific features for testing different configurations
- Test feature combinations that may not be enabled in the main library
- Ensure comprehensive coverage across all feature combinations

### Avoiding Circular Dependencies
The separate crate structure prevents circular dependencies between `storage-core` and `storage-core-test`:
- `storage-core` provides the core storage functionality
- `storage-core-test` provides testing utilities and depends on `storage-core`
- `storage-core-integration-tests` depends on both crates for comprehensive testing

This architecture allows `storage-core-test` to depend on `storage-core` without creating a circular dependency, enabling the test utilities to be used by the integration tests.

## Test Structure

The integration tests cover:
- **File Object operations** - CRUD operations, metadata handling
- **File Folder operations** - Directory management, listing
- **Capabilities** - Service capability discovery
- **Versioning** - Object versioning functionality
- **OpenAPI** - REST API specification validation

## Running Tests

```bash
# Run the default workspace profile: all non-integration tests plus the
# historical small integration subset
cargo nextest run --workspace

# Run only storage-core-integration-tests at a chosen cumulative tier
cargo nextest run -p storage-core-integration-tests -P integration-small
cargo nextest run -p storage-core-integration-tests -P integration-medium
cargo nextest run -p storage-core-integration-tests -P integration-large
cargo nextest run -p storage-core-integration-tests -P integration-full

# Run a specific integration test target within the selected profile
cargo nextest run -p storage-core-integration-tests --test fileobject -P integration-medium
```

### Nextest Profiles

The integration suite is selected with nextest profiles in `.config/nextest.toml`.
The profiles are cumulative:

- `integration-small` preserves the previous default smoke subset.
- `integration-medium` adds targeted edge-case, cache, list, and local-service coverage.
- `integration-large` adds most broad integration coverage while excluding the largest matrices.
- `integration-full` runs every compiled test.

Workspace runs still include tests from all other crates. The profile filters only reduce
`storage-core-integration-tests`.

### Local Services

MinIO, the nginx proxy, and Valkey can be started locally with:

> The compose file that starts these services is not part of the published
> source: it builds against an internal container registry. Start MinIO with
> the command the test suite prints when it skips those tests, and supply
> Valkey and a proxy yourself.

Use `OMNI_STORAGE_SERVICE_MINIO_ADDRESS=127.0.0.1:9000` for local MinIO and
`UCC_PROXY=https://127.0.0.1:8443 UCC_PROXY_SSL_VERIFYCERT=insecure` for proxy runs.
`UCC_PROXY_SSL_VERIFYCERT=insecure` turns off TLS certificate and hostname verification for
connections through the proxy, so use it only with a local test proxy such as the nginx one
above. It is the only value that does: any other value, including `false` and `0`, keeps
verification on.
Cloud-backed tests still require the credentials loaded by `tools/vault/local_env.sh`.

## Dependencies

- `storage-core` - The main storage library being tested
- `storage-core-test` - Testing utilities and mock implementations
- Various testing frameworks and HTTP clients for comprehensive test coverage

## Integration Test Timing Baseline

Baseline captured on 2026-05-08 with:

- `cargo-nextest 0.9.133`
- `rustc 1.95.0`
- 48 logical CPUs
- MinIO, Valkey and an HTTP proxy running locally
- `OMNI_STORAGE_SERVICE_MINIO_ADDRESS=127.0.0.1:9000`
- `OMNI_STORAGE_SERVICE_MINIO_REQUIRED=true`
- cloud credentials loaded for S3, Azure, GCP S3-compatible, and CDN tests

The profile filters are cumulative. Workspace counts include all tests outside
`storage-core-integration-tests`; integration counts are for that crate only.
Counts were refreshed on 2026-05-14 with `cargo nextest list`; wall-time and
percentile values are still the 2026-05-08 timing baseline and were not rerun.

| Profile | Integration Cases | Workspace Cases | Wall Time | p50 | p95 | p99 | Max |
|---|---:|---:|---:|---:|---:|---:|---:|
| `integration-small` | 1,106 | 1,526 | 40.6s | 1.35s | 3.44s | 7.37s | 23.72s |
| `integration-medium` | 2,012 | 2,432 | 56.1s | 1.32s | 3.25s | 5.50s | 21.06s |
| `integration-large` | 3,632 | 4,052 | 156.1s | 1.68s | 3.77s | 5.14s | 19.04s |
| `integration-full` | 7,964 | 8,384 | not run locally | - | - | - | - |

`integration-full` is count-verified and configured for CI. It adds the largest
matrices, including full stat/read/multipart, CDN body, and manifest
`read_from_address_full` coverage, so it should normally be run sharded.

For local runs, start MinIO, Redis/Valkey, and the nginx proxy with:

> The compose file that starts these services is not part of the published
> source: it builds against an internal container registry. Start MinIO with
> the command the test suite prints when it skips those tests, and supply
> Valkey and a proxy yourself.

Timing command used:

```sh
NEXTEST_EXPERIMENTAL_LIBTEST_JSON=1 \
OMNI_STORAGE_SERVICE_MINIO_ADDRESS=127.0.0.1:9000 \
OMNI_STORAGE_SERVICE_MINIO_REQUIRED=true \
OMNI_STORAGE_SERVICE_REDIS_ADDRESS=127.0.0.1:6379 \
cargo nextest run -p storage-core-integration-tests -P integration-large \
  --message-format libtest-json-plus \
  --status-level fail \
  --final-status-level fail \
  --no-fail-fast \
  > /tmp/storage-core-integration-large.jsonl
```
