# Storage Core — component map

> **Applies to:** `storage/reference/storage-core` (Rust library workspace behind the Storage Service) · **Released:** no separate artifact; the crates are pre-1.0 alpha libraries with no release of their own; released Storage Service **1.2.3** records the crate versions it uses in its [`CHANGELOG.md`](../../../../storage/reference/storage-service/CHANGELOG.md) `[1.2.3]` · **This repo's snapshot:** the source commit in [`SOURCE_VERSIONS.md`](../../../../SOURCE_VERSIONS.md). The crate `Cargo.toml` files still read the versions 1.2.3 shipped with, but the code contains unreleased changes (see [Released versus snapshot](#released-versus-snapshot)), and the Storage Service snapshot's `Cargo.lock` records newer storage-core and storage-core-builder versions. `rust-version = "1.91.1"`; vendored Storage API protos pinned to storage-protos `0.28.0` + local patches · **Sources:** [`storage/reference/storage-core/`](../../../../storage/reference/storage-core/README.md), upstream `storage-core` skill (the [source skill package](../../NOTICE.md) pin)

This page is a table of contents. Read the linked doc before answering a design question. For a running service (Helm, deployment, configuration of a cluster) use [storage-service.md](storage-service.md); for the wire contract use [storage-api.md](storage-api.md).

## What it is

A Rust framework, not a deployable service. It serves the Storage API over gRPC (tonic 0.14) and REST (axum 0.8) from one implementation, with S3 and Azure Blob backends and composable layers (stat/list/small-object/metadata caches, egress throttle, metrics, additional headers, manifest, metadata key remap/route). Writing on it: [develop/custom-storage-adapter/storage-core-rust.md](../develop/custom-storage-adapter/storage-core-rust.md).

## Crates

| Crate | Version | Holds |
|---|---|---|
| [`storage-core`](../../../../storage/reference/storage-core/storage-core/CHANGELOG.md) | `Cargo.toml` (the Storage Service lock records a newer version) | Backends, layers, `StorageService` server; feature flags per backend/layer (`full` = all) |
| [`storage-core-builder`](../../../../storage/reference/storage-core/storage-core-builder/CHANGELOG.md) | `Cargo.toml` (the Storage Service lock records a newer version) | Per-API traits (`traits::{fileobject,filefolder,capabilities,versioning}::{v1alpha,v1beta}`, `metadata::v1alpha`), gRPC/REST ingresses, service builders |
| [`storage-core-protos`](../../../../storage/reference/storage-core/storage-core-protos/README.md) | `Cargo.toml` | Generated bindings from the vendored protos (vendored `protoc`) |
| [`storage-core-types`](../../../../storage/reference/storage-core/storage-core-types/CHANGELOG.md) | `Cargo.toml` | Shared types |
| [`storage-core-util`](../../../../storage/reference/storage-core/storage-core-util/CHANGELOG.md) | `Cargo.toml` | Utilities, secrets, caching primitives |
| [`storage-core-test`](../../../../storage/reference/storage-core/storage-core-test/README.md) | `Cargo.toml` | Test clients and harness |
| [`storage-core-integration-tests`](../../../../storage/reference/storage-core/storage-core-integration-tests/README.md) | 0.4.1 | Integration suite against real backends |

## Route by task

| Question | Go to |
|---|---|
| How the pieces fit; request path | [`docs/architecture.md`](../../../../storage/reference/storage-core/docs/architecture.md) |
| Writing a backend | [`docs/backends.md`](../../../../storage/reference/storage-core/docs/backends.md) |
| Trait method signatures, where per-request context goes, adding a method | [`docs/architecture.md` "Trait method conventions"](../../../../storage/reference/storage-core/docs/architecture.md#trait-method-conventions) |
| Writing or ordering layers | [`docs/layers.md`](../../../../storage/reference/storage-core/docs/layers.md) |
| Building an application, layer setup, multiple services | [`docs/applications.md`](../../../../storage/reference/storage-core/docs/applications.md) |
| Configuration shape | [`docs/configuration.md`](../../../../storage/reference/storage-core/docs/configuration.md) |
| Credentials, S3/MinIO/GCS endpoint styles, Kubernetes secret mounting | [`docs/secrets.md`](../../../../storage/reference/storage-core/docs/secrets.md) |
| Errors per RPC | [`docs/errors.md`](../../../../storage/reference/storage-core/docs/errors.md) |
| Metric names, example dashboard | [`docs/metrics/metrics.md`](../../../../storage/reference/storage-core/docs/metrics/metrics.md), [`example_dashboard.json`](../../../../storage/reference/storage-core/docs/metrics/example_dashboard.json) |
| Worked examples | [`examples/`](../../../../storage/reference/storage-core/examples/): `basic` (bins `basic-azure`, `basic-s3`), `boxed-builders`, `greeter_service` (a new service), `manifest`, `remapper`, `traffic-generator` |
| Vendored proto pin and the local-patch warning | [`storage-core-protos/proto/storage-protos/VERSION`](../../../../storage/reference/storage-core/storage-core-protos/proto/storage-protos/VERSION) |
| Integration test tiers | [test/conformance.md](../test/conformance.md#storage-core-integration-tests) |
| Lint, deny, nextest config | [`clippy.toml`](../../../../storage/reference/storage-core/clippy.toml), [`deny.toml`](../../../../storage/reference/storage-core/deny.toml), [`.config/nextest.toml`](../../../../storage/reference/storage-core/.config/nextest.toml) |
| Azure SDK fork it patches in | [`storage/reference/externals/azure-sdk/`](../../../../storage/reference/externals/azure-sdk/) (`[patch.crates-io]` in [`Cargo.toml`](../../../../storage/reference/storage-core/Cargo.toml)) |

## Released versus snapshot

This snapshot keeps the Storage Core version numbers that released Storage Service 1.2.3 was built on, but adds changes that are not in that release. The Storage Service `[Unreleased]` section records them as a Storage Core update. Neither crate's `CHANGELOG.md` has a new heading for them; the builder lists the write change under its existing heading.

| Change | Effect | Source |
|---|---|---|
| File-object writes read exactly `data_object_size` bytes (gRPC and REST, v1alpha and v1beta) | A shorter body fails with `INVALID_ARGUMENT`/400; extra bytes are discarded silently, so an oversized body is stored truncated. Prevents unbounded buffering of oversized request bodies. The declared `data_object_size` itself is not capped | [`storage-core-builder/src/api_services/fileobject/utils.rs`](../../../../storage/reference/storage-core/storage-core-builder/src/api_services/fileobject/utils.rs), `storage-core-builder/CHANGELOG.md` |
| **Breaking for implementers:** the `Versioning` trait's `is_versioning_enabled(...) -> Result<bool>` is gone. v1alpha has `is_versioned(&self, request: Request<IsVersionedRequest>) -> Result<IsVersionedResponse>`; v1beta has no replacement (there is no v1beta `IsVersioned` RPC) | Backends and layers that implemented `is_versioning_enabled` no longer compile. The gRPC and REST `IsVersioned` handlers pass the caller's whole request (metadata and extensions) to the trait | [`storage-core-builder/src/traits/versioning.rs`](../../../../storage/reference/storage-core/storage-core-builder/src/traits/versioning.rs), [`docs/architecture.md`](../../../../storage/reference/storage-core/docs/architecture.md#trait-method-conventions) |
| Trait method shape enforced at compile time | `generate_storage_trait!` requires `request: Request<_>` as the first parameter after `&self`; only `StreamingOptions`, `BoxFuture<Bytes>` and the grandfathered `version_id_token: Option<String>` may follow. Per-request context (`AdditionalHeaders`, `CacheControl`, cached `ResourceInfo`) travels in `request.extensions()` | [`storage-core-builder/src/traits/mod.rs`](../../../../storage/reference/storage-core/storage-core-builder/src/traits/mod.rs), [`docs/architecture.md`](../../../../storage/reference/storage-core/docs/architecture.md#trait-method-conventions) |
| New test `no_fabricated_request_context` | Fails the build's tests if non-test code in `storage-core/src` or the gRPC handlers creates an empty `MetadataMap`, `Extensions` or `Request`, which would drop the caller's credentials | [`storage-core/tests/no_fabricated_request_context.rs`](../../../../storage/reference/storage-core/storage-core/tests/no_fabricated_request_context.rs) |
| Strict-mode `IsVersioned` | With `allow_unconfigured_endpoints = false`, `IsVersioned` for an endpoint with no secrets entry fails with `NOT_FOUND` unless a backend-wide `s3.versioning_enabled` / `azure.versioning_enabled` is set | [`docs/secrets.md`](../../../../storage/reference/storage-core/docs/secrets.md) |
| Manifest layer `IsVersioned`/`IsReadOnly` | A cold `.manifest` read now carries the caller's metadata, so a permission-enforcing resolver authorizes it as the caller. The warm manifest cache is keyed by workspace path, not principal, so later callers are answered from cache **without** a permission check on the `.manifest` | [`storage-core/src/manifest/README.md`](../../../../storage/reference/storage-core/storage-core/src/manifest/README.md) |
| Dependency advisories | `rustls` 0.23.46; `deny.toml` ignores RUSTSEC-2026-0349 and RUSTSEC-2026-0346 for the AWS SDK's `rustls` 0.21.12 and RUSTSEC-2024-0449 (`unsafe-libyaml`, unmaintained) with stated rationale | [`deny.toml`](../../../../storage/reference/storage-core/deny.toml), `Cargo.lock` |

Unreleased Storage Service changes that are not in Storage Core (URL normalization by default, permission retries, header forwarding) are implemented in the Storage Service crate.

## Differences from the upstream skill and defects

- The upstream skill says crates publish to an internal Cargo registry and consumers pin exact versions. Here the Storage Service consumes them by path (`../storage-core/...`, recorded as a layout adaptation in `SOURCE_VERSIONS.md`); registry publication is not verifiable in this repo.
- [`README.md`](../../../../storage/reference/storage-core/README.md) has no build or test commands. Use `cargo build -p storage-core --features full` and `cargo nextest run --workspace` from the workspace root.
- [`examples/basic/README.md`](../../../../storage/reference/storage-core/examples/basic/README.md) describes single-feature builds with `azure` as default; the crate enables both features and builds two binaries. It also contains a remapper section copied from [`examples/remapper/`](../../../../storage/reference/storage-core/examples/remapper/README.md).
- [`docs/backends.md`](../../../../storage/reference/storage-core/docs/backends.md#complete-example) complete example does not compile (`.await` in a non-async `new`) and joins caller paths without traversal checks.
- [`storage-core-integration-tests/README.md`](../../../../storage/reference/storage-core/storage-core-integration-tests/README.md) now says the local services compose file is not published and tells you to start MinIO with the command the suite prints, and to supply Valkey and a proxy yourself. The same README and the fixtures' error messages (`src/fixture.rs`, `src/lib.rs`) tell you to `source tools/vault/local_env.sh` for cloud credentials; that script is not in this snapshot (`tools/` has only `manifests/` and `osrb/`). Export the `OMNI_STORAGE_SERVICE_*` variables the fixtures read from your own secret store.
- The `storage-core-protos` [`VERSION`](../../../../storage/reference/storage-core/storage-core-protos/proto/storage-protos/VERSION) note now says the locally patched `IsReadOnly`/`IsVersioned` RPCs were added upstream in unreleased Storage API source, but the crate has not been re-vendored: the rest of the tree is still storage-protos `0.28.0` plus the partial native-metadata pin.
- The v1beta REST OpenAPI annotation for `complete_redirect_upload` in [`storage-core-builder/src/api_services/fileobject/v1beta/rest.rs`](../../../../storage/reference/storage-core/storage-core-builder/src/api_services/fileobject/v1beta/rest.rs) declares path `/multipart/complete`; the route itself is `/redirect/complete`. The served OpenAPI document for v1beta can therefore show the wrong path.
- The vendored v1alpha Metadata proto lacks `GetMetadataBatch`, `QueryMetadata` and `DeleteMetadataBatch`; the Storage Service compiles its own complete copy.

Related: [storage-api.md](storage-api.md), [storage-service.md](storage-service.md), [common-libraries.md](common-libraries.md).
