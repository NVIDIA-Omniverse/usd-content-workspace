# Path B: production implementation in any language

> **Applies to:** a from-scratch Storage API service in Go, Java, Rust, C++ or another language · **Released:** contract `storage-api` **1.0.0-beta.4** · **This repo's snapshot:** Storage API **unreleased (after 1.0.0-beta.4)** ([`storage/api/CHANGELOG.md`](../../../../../storage/api/CHANGELOG.md)) · **Sources:** [`templates/production_impl/README.md`](../../../../../storage/api/templates/production_impl/README.md), [`AGENTS.md`](../../../../../storage/api/AGENTS.md), [`storage/api/proto/`](../../../../../storage/api/proto/), [`storage/api/openapi/`](../../../../../storage/api/openapi/), conformance `.feature` files, the [source skill package](../../../NOTICE.md)

Use this path when the Python reference is not the right runtime. You own the transport, the backend abstraction and the operational properties. The conformance suite is still the acceptance test.

## Generate code from the contract

Generate from the exact definitions you target (released beta.4 or this snapshot). Every `.proto` sets `go_package`, `java_package` (`com.nvidia.omniverse.storage.<group>.<version>`) and `java_multiple_files`. All Go packages are named `storage` under `pb.api.nvidia.com/omniverse/storage/<group>/<version>`; map them into your module with `--go_opt=M...` or `module=` options.

```bash
cd storage/api
# Go: the import root must be proto/, and zsh or bash with `shopt -s globstar` is needed for **
protoc -I proto \
  --go_out=./gen --go_opt=paths=source_relative \
  --go-grpc_out=./gen --go-grpc_opt=paths=source_relative \
  proto/nvidia/omniverse/storage/**/*.proto
```

```rust
// Rust build.rs, matching the tonic 0.14 line used by storage-core (tonic-prost-build)
fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_prost_build::configure().build_server(true).compile_protos(
        &[
            "proto/nvidia/omniverse/storage/fileobject/v1beta/fileobject_service.proto",
            "proto/nvidia/omniverse/storage/filefolder/v1beta/filefolder_service.proto",
            "proto/nvidia/omniverse/storage/capabilities/v1beta/capabilities.proto",
            "proto/nvidia/omniverse/storage/versioning/v1beta/versioning.proto",
        ],
        &["proto"],
    )?;
    Ok(())
}
```

The commands in [`AGENTS.md`](../../../../../storage/api/AGENTS.md) and [`README.md`](../../../../../storage/api/README.md) (`protoc --go_out=. --go-grpc_out=. proto/nvidia/omniverse/storage/**/*.proto`) omit `-I proto`; the imports (`nvidia/omniverse/storage/...`) do not resolve without it. The Rust and Java skeletons in [`templates/production_impl/README.md`](../../../../../storage/api/templates/production_impl/README.md) pin older toolchains (tonic 0.10 / prost 0.12; grpc-java 1.59, protobuf 3.24); treat them as structure, not as version advice. Rust services can instead build on Storage Core ([storage-core-rust.md](storage-core-rust.md)).

REST: either generate from the OpenAPI files (snapshot files parse; released beta.4 files do not) or hand-write routes that serve the same paths and bodies ([apis/storage.md](../../apis/storage.md#rest-endpoints)). The suite can diff your served OpenAPI against the reference with `oasdiff` ([test/conformance.md](../../test/conformance.md#storage-api-conformance-suite)).

## Implementation checklist (v1beta)

The required set is the v1beta services. The template and `AGENTS.md` list `CreateFolder` as required under v1beta; it exists only in v1alpha.

- **CapabilitiesService**: `ListServices` (exactly the groups and versions you serve), `ListTopLevelAddresses`.
- **FileObjectService**: `Stat`, `Read`, `ReadFromAddress`, `Write` (bidirectional), `FetchWriteTypeInfo`, `CompleteRedirectUpload`, `UploadPart`, `CompleteMultipartUpload`, `AbortMultipartUpload`, `Delete`, `Enumerate` (recursive).
- **FileFolderService**: `List`, `ListStat`, `DeleteFolder` (must fail on a non-empty folder).
- **VersioningService**: `EnumerateVersions`. An unversioned backend still answers with the current version.
- **REST**: the matching OpenAPI routes, including the `x-nvidia-omniverse-storage-metadata` and `x-nvidia-omniverse-storage-resource-identity` headers on Stat and reads, and 300 responses for redirects.
- Optional v1alpha surface: [capabilities-and-versioning.md](capabilities-and-versioning.md#what-to-serve).

## Error mapping

From [`templates/production_impl/README.md`](../../../../../storage/api/templates/production_impl/README.md) and the reference service; the `.feature` files decide per operation.

| Condition | gRPC | HTTP |
|---|---|---|
| Not found (address, identity, upload session) | `NOT_FOUND` | 404 |
| Caller not allowed | `PERMISSION_DENIED` | 403 |
| Malformed address, identity, parameter | `INVALID_ARGUMENT` | 400 |
| Already exists (`fail_if_exists`, folder create) | `ALREADY_EXISTS` | 409 |
| `previous_version` mismatch, read-only storage | `FAILED_PRECONDITION` | 412 |
| Unsupported operation (for example non-empty native metadata on an unsupported route) | `UNIMPLEMENTED` | 501 |
| Backend failure | `INTERNAL` / `UNAVAILABLE` | 500 / 503 |

Idempotency is part of the contract: [`features/fileobject/v1beta/delete.feature`](../../../../../storage/api/conformance_tests/src/conformance_tests/features/fileobject/v1beta/delete.feature) requires `Delete` of a nonexistent object to return `OK`/204, and a `previous_version` mismatch to return `FAILED_PRECONDITION`/412 without deleting.

## Design decisions

### Addresses and identities

- Addresses: URIs your service recognizes as its own (for file-like stores, RFC 3986 so clients can append relative paths). Reject anything outside your top-level addresses with `INVALID_ARGUMENT`.
- Identities: opaque, URL-safe (for example base64url of a small JSON document), self-contained enough to locate the exact version, and stable across service restarts. An identity is a locator, **not** an authorization token: authorize every read by the decoded location, not by possession of the identity.
- Bound sizes: the OpenAPI files cap addresses and identities at `maxLength: 4096`. Keep encoded identities well under that so they fit in a URL path and a response header.
- Normalize paths deterministically. Collapsing `//` or resolving `..` changes which object an address names on flat object stores (see the Storage Service's [`url-normalizer.rst`](../../../../../storage/reference/storage-service/docs/helm/url-normalizer.rst)); decide once and document it.

### Versioning without native support

From the template: version-suffixed keys, a separate version manifest, or a single current version. If you choose a single version, advertise it (v1alpha `IsVersioned` returns `false`) and make superseded identities return `NOT_FOUND`/404, which the current conformance suite enforces for unversioned storage. See [capabilities-and-versioning.md](capabilities-and-versioning.md#versioned-unversioned-and-read-only-storage).

### Large objects

Offer redirect or multipart uploads for large objects so bytes go straight to the backend (presigned URLs, SAS tokens). Advertise size thresholds through `FetchWriteTypeInfo`. Keep presigned URL lifetimes short, scope them to one object and method, and never log them.

### Streaming

`Read`, `ReadFromAddress`, `Enumerate`, `List`, `ListStat` and `EnumerateVersions` stream. Use bounded chunk sizes (the template suggests 64 KiB to 1 MiB), respect backpressure, and stop work when the client cancels.

## Operational requirements before production

- TLS on every listener reachable outside the pod, or terminate TLS at the ingress and keep the pod port cluster-internal.
- Authentication and authorization through the stack's Envoy auth extension and Permission Service, or an equivalent you can show to a security reviewer ([develop/add-your-service.md](../add-your-service.md#authentication-and-authorization)).
- gRPC health service, readiness/liveness probes, OpenTelemetry metrics and traces ([components/common-libraries.md](../../components/common-libraries.md)).
- The Discovery capabilities API so the Auto Discovery operator can register the adapter ([develop/add-your-service.md](../add-your-service.md#expose-discovery-capabilities)).
- Conformance in a pre-production environment against a disposable prefix ([test/conformance.md](../../test/conformance.md#storage-api-conformance-suite)).

Packaging and deployment: [packaging-and-deploying.md](packaging-and-deploying.md).
