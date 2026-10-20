(discovery-capabilities-api-index-index)=
# Omniverse API Discovery

(discovery-capabilities-api-index-introduction)=
## Introduction

API discovery lets clients find which deployed service implements a given REST resource version or gRPC service, and where to reach it, without a fixed catalog of service types. These documents specify two contracts that together make this possible:

- The **Discovery Catalog API** is the contract a discovery service exposes to clients. `GET /api/v2beta/services` (also served at `/api/v2/services` for existing clients) returns one JSON catalog that lists each discovered service, its REST and gRPC URLs, and the APIs it implements. Implement this contract to build a discovery service that existing clients can use.
- The **Capabilities API** is the contract each target service exposes so that a discovery service can learn what it implements. It has two independently versioned representations:
  - REST `v1beta`, defined by the OpenAPI document and exposed as `GET /v1beta/services`.
  - gRPC `v2beta`, defined by the `nvidia.omniverse.discovery.capabilities.v2beta` Protocol Buffer package and exposed through `CapabilitiesService.ListServices`.

The two Capabilities API representations serve the same purpose, but their field names and pagination support follow their own source specifications. Callers must select the contract version they implement rather than treating the two versions as interchangeable wire formats.

The machine-readable contracts are published alongside these docs as `discovery-openapi.yaml` (Discovery Catalog API), `openapi.yaml` (Capabilities REST API), and `capabilities.proto` with `capabilities_service.proto` (Capabilities gRPC API).

```{toctree}
:maxdepth: 1
:caption: Contents:

ecosystem-usage
discovery-catalog-api
rest-api
grpc
references
```

(discovery-capabilities-api-index-terms-and-abbreviations)=
## Terms and Abbreviations

| Term | Meaning |
|---|---|
| Capability | A REST resource version or fully qualified gRPC service that a server advertises. |
| Catalog | The JSON document returned by the Discovery Catalog API that lists discovered services and their capabilities. |
| Discovery client | A caller that obtains capability information and uses it to locate a compatible API. |
| Discovery service | A service that queries target services' Capabilities API and serves the combined catalog. |
| Endpoint | A named REST resource or fully qualified gRPC service exposed by a server. |
| REST | Representational State Transfer over HTTP with JSON payloads in this contract. |
| RPC | Remote procedure call. |
| Target service | A service that implements the Capabilities API and is listed in a catalog. |

(discovery-capabilities-api-index-see-next)=
## See next

- {ref}`Ecosystem usage <discovery-capabilities-api-ecosystem-usage-ecosystem-usage>`
- {ref}`Discovery Catalog API v2beta <discovery-capabilities-api-discovery-catalog-api-discovery-catalog-api-v2beta>`
- {ref}`Capabilities REST API v1beta <discovery-capabilities-api-rest-api-rest-api-v1beta>`
- {ref}`Capabilities gRPC API v2beta <discovery-capabilities-api-grpc-grpc-api-v2beta>`
