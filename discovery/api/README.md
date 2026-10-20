# discovery-protos

Canonical Protocol Buffer definitions for the USD Storage **API Discovery** services.

This repository is the single source of truth for the discovery gRPC/REST API contract. It contains only `.proto` sources — no generated code is committed here. Downstream services copy or reference these protos and compile stubs locally.

> [!IMPORTANT]
> The current latest version is **`v2beta`** (`nvidia.omniverse.discovery.capabilities.v2beta`). New integrations should use it.
>
> **`v2alpha` is deprecated** and will be removed in a future release. Existing consumers should migrate to `v2beta`.

## Layout

Proto files live under `proto/`, with each file's directory mirroring its package name (buf's `PACKAGE_DIRECTORY_MATCH` rule):

```
proto/nvidia/omniverse/discovery/capabilities/
├── v2alpha/
│   ├── capabilities.proto
│   └── capabilities_service.proto
└── v2beta/
    ├── capabilities.proto
    └── capabilities_service.proto
```

Each version is a self-contained package; versions never import from one another.

`buf.yaml` declares `proto` as the module root and enables the `STANDARD` lint and `FILE` breaking-change rule sets.

## The Capabilities API

Packages: `nvidia.omniverse.discovery.capabilities.v2alpha` and `nvidia.omniverse.discovery.capabilities.v2beta` (currently identical apart from the package name and the deprecation markers on `v2alpha`). `capabilities_service.proto` defines the service and its request/response messages; `capabilities.proto` defines the supporting types.

`CapabilitiesService` lets a client reflect on what a storage server implements:

| RPC | Purpose |
| --- | --- |
| `ListServices` | All service APIs implemented by the server, with gRPC and REST transport details per service. |

Message hierarchy for `ListServices`:

```
ListServicesResponse
└── ServiceEntry (service_name + transports)
    ├── GrpcService → GrpcEndpoint (fully-qualified gRPC service name)
    └── RestService → RestEndpoint → RestVersion → RestVersionDetail (URL path)
```

## Consumers

Downstream Rust services copy the proto files for the version they use into their own tree and compile it at build time with `tonic-prost-build` in `build.rs`:

- `discovery-service`
- `auto-discovery-service`

**When this proto changes, the copy in each consumer repo must be updated manually.** There is no automated propagation.

## Working with the protos

Requires [`buf`](https://buf.build/docs/installation) (and `protoc` for downstream compilation).

```bash
# Lint
buf lint

# Check for breaking changes against main
buf breaking --against '.git#branch=main'

# Format in-place
buf format -w
```

## Conventions

- **Package:** `nvidia.omniverse.discovery.<service>.v<N>` — always keep the version suffix.
- **Go option:** `pb.api.nvidia.com/omniverse/discovery/<service>/v<N>;<short-alias>`
- **Java options:** `java_multiple_files = true`, `java_outer_classname` set, `java_package` mirrors the proto package in Java style.
- **Field numbers are permanent** — never reuse or delete a field number without a `reserved` statement.
- **AIP linter suppressions** (`-- api-linter: ... --`) are acceptable when documented with an `aip.dev/not-precedent` justification.

## License

Licensed under the [Apache License, Version 2.0](LICENSE.txt). See the SPDX headers in each source file.
