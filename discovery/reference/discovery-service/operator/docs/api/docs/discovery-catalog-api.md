(discovery-capabilities-api-discovery-catalog-api-discovery-catalog-api-v2beta)=
# Discovery Catalog API v2beta

The Discovery Catalog API is the client-facing contract of a discovery service. A discovery service queries the {ref}`Capabilities API <discovery-capabilities-api-rest-api-rest-api-v1beta>` of each target service it knows about, combines the results with the network location of each target, and returns the combined catalog as one JSON document. A client reads the catalog to find which service implements a given REST resource version or gRPC service, and where to reach it.

This document specifies the catalog response format and the rules for building catalog entries from Capabilities API responses, so that independently built discovery services return interchangeable catalogs. The machine-readable contract is published alongside these docs as `discovery-openapi.yaml`.

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this section are to be interpreted as described in [RFC 2119](https://datatracker.ietf.org/doc/html/rfc2119).

(discovery-capabilities-api-discovery-catalog-api-error-codes)=
## Error Codes

| HTTP status | Error code | Condition |
|---|---|---|
| `200 OK` | — | The catalog is returned. An empty catalog is a `200 OK` with an empty `services` array. |
| `5xx` | — | The implementation cannot produce a catalog. |

- An implementation MUST return `200 OK` whenever it can produce a catalog, including when the catalog is empty.
- An implementation MUST NOT return `404 Not Found` or any other error to signal that no services were discovered.
- The contract defines no error response body. Clients MUST NOT depend on the body of a non-`200` response.
- An implementation MAY require authentication. The authentication mechanism, and its `401`/`403` responses, are outside this contract.

(discovery-capabilities-api-discovery-catalog-api-general-rules)=
## General Rules

- The response body MUST be a JSON object and MUST be sent with `Content-Type: application/json`.
- Member names are case-sensitive and MUST be spelled exactly as specified. Note that `schema-version` uses a hyphen while all other multi-word members use an underscore (`capability_source`).
- Optional members that have no value MUST be omitted. An implementation MUST NOT send `null` for any member.
- Clients MUST ignore members they do not recognize, so that optional members can be added to the schema without a `schema-version` change.
- An implementation SHOULD sort `services` by `name`, each `rest.endpoints` array by `name`, and each `grpc.endpoints` array by `service`, using byte-wise ascending order, so that equal catalogs serialize identically. Clients MUST NOT depend on array order or on the member order of JSON objects.
- An implementation SHOULD send a `Cache-Control` header with a `max-age` that reflects how often the catalog is refreshed (for example `Cache-Control: max-age=30, must-revalidate`). Clients MAY cache the catalog for the period the header allows.

(discovery-capabilities-api-discovery-catalog-api-response-models)=
## Response Models

(discovery-capabilities-api-discovery-catalog-api-services-v2-response)=
### ServicesV2Response

| Field | Type | Required | Description |
|---|---|---:|---|
| `schema-version` | integer | Yes | Catalog schema version. MUST be `2`. Clients MUST reject a response whose `schema-version` is not `2`. |
| `services` | array of {ref}`DiscoveredService <discovery-capabilities-api-discovery-catalog-api-discovered-service>` | Yes | Discovered services. MAY be empty. Each `name` MUST be unique within the array. |

(discovery-capabilities-api-discovery-catalog-api-discovered-service)=
### DiscoveredService

| Field | Type | Required | Description |
|---|---|---:|---|
| `name` | string | Yes | Non-empty identifier of the target service, unique within the catalog. Assigned by the discovery service; see {ref}`Service identity <discovery-capabilities-api-discovery-catalog-api-service-identity>`. |
| `capability_source` | string | Yes | Capabilities API representation the entry was built from: `grpc` (gRPC `ListServices`) or `openapi` (REST `GET /v1beta/services`). |
| `rest` | {ref}`RestTransport <discovery-capabilities-api-discovery-catalog-api-rest-transport>` | No | REST location and resources. Omitted when the target has no REST URL for the catalog's audience or advertises no REST resources. |
| `grpc` | {ref}`GrpcTransport <discovery-capabilities-api-discovery-catalog-api-grpc-transport>` | No | gRPC location and services. Omitted when the target has no gRPC URL for the catalog's audience or advertises no gRPC services. |

A `DiscoveredService` MAY contain neither `rest` nor `grpc`. Clients MUST treat such an entry as offering no usable API.

(discovery-capabilities-api-discovery-catalog-api-rest-transport)=
### RestTransport

| Field | Type | Required | Description |
|---|---|---:|---|
| `url` | string | Yes | Absolute base URL for REST calls to the target service, without a trailing `/`. MAY include a path prefix. |
| `endpoints` | array of {ref}`RestEndpoint <discovery-capabilities-api-discovery-catalog-api-rest-endpoint>` | Yes | Named REST resources. MUST contain at least one entry. Each `name` MUST be unique within the array. |

(discovery-capabilities-api-discovery-catalog-api-rest-endpoint)=
### RestEndpoint

| Field | Type | Required | Description |
|---|---|---:|---|
| `name` | string | Yes | Non-empty REST resource name, for example `fileobject`. |
| `versions` | object | Yes | Map from version identifier (for example `v1`, `v1alpha`) to {ref}`VersionDetail <discovery-capabilities-api-discovery-catalog-api-version-detail>`. MUST contain at least one member. Version identifiers MUST be non-empty. |

`versions` is a JSON object keyed by version identifier, not an array. This differs from the Capabilities API, where `versions` is an array of version objects.

(discovery-capabilities-api-discovery-catalog-api-version-detail)=
### VersionDetail

| Field | Type | Required | Description |
|---|---|---:|---|
| `path` | string | No | Path of the resource version, beginning with `/`, relative to the REST `url`. Omitted when the target service does not report one. |

A `VersionDetail` with no `path` is the empty object `{}`. It states that the version is supported without stating where it is.

(discovery-capabilities-api-discovery-catalog-api-grpc-transport)=
### GrpcTransport

| Field | Type | Required | Description |
|---|---|---:|---|
| `url` | string | Yes | Absolute gRPC target URL. The `https` scheme indicates TLS; the `http` scheme indicates plaintext. |
| `endpoints` | array of {ref}`GrpcServiceEndpoint <discovery-capabilities-api-discovery-catalog-api-grpc-service-endpoint>` | Yes | gRPC services. MUST contain at least one entry. Each `service` MUST be unique within the array. |

(discovery-capabilities-api-discovery-catalog-api-grpc-service-endpoint)=
### GrpcServiceEndpoint

| Field | Type | Required | Description |
|---|---|---:|---|
| `service` | string | Yes | Non-empty fully qualified Protocol Buffer service name, for example `nvidia.omniverse.storage.fileobject.v1.FileObjectService`. |

(discovery-capabilities-api-discovery-catalog-api-get-api-v2beta-services)=
## GET /api/v2beta/services — List Discovered Services

Returns the current catalog. The operation takes no path parameters, query parameters, or request body. A client calls it at startup and again whenever its cached copy expires, then selects a target by REST resource and version or by gRPC service name.

An implementation SHOULD also serve the identical response at `GET /api/v2/services`, the path used before v2beta, so that existing clients keep working. New clients SHOULD call `/api/v2beta/services` and MAY fall back to `/api/v2/services` when a discovery service returns `404 Not Found` for the v2beta path.

**Response:** {ref}`ServicesV2Response <discovery-capabilities-api-discovery-catalog-api-services-v2-response>`

(discovery-capabilities-api-discovery-catalog-api-get-api-v2beta-services-example)=
### Example

```console
curl --request GET \
  --header 'Accept: application/json' \
  https://discovery.example.com/api/v2beta/services
```

```text
HTTP/1.1 200 OK
Content-Type: application/json
Cache-Control: max-age=30, must-revalidate
```

```json
{
  "schema-version": 2,
  "services": [
    {
      "name": "search",
      "capability_source": "grpc",
      "grpc": {
        "url": "https://grpc.search.example.com",
        "endpoints": [
          {
            "service": "nvidia.omniverse.search.v1.SearchService"
          }
        ]
      }
    },
    {
      "name": "storage",
      "capability_source": "grpc",
      "rest": {
        "url": "https://storage.example.com",
        "endpoints": [
          {
            "name": "fileobject",
            "versions": {
              "v1": {
                "path": "/api/v1/fileobject"
              },
              "v1alpha": {}
            }
          }
        ]
      },
      "grpc": {
        "url": "https://grpc.storage.example.com",
        "endpoints": [
          {
            "service": "nvidia.omniverse.storage.fileobject.v1.FileObjectService"
          },
          {
            "service": "nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService"
          }
        ]
      }
    }
  ]
}
```

(discovery-capabilities-api-discovery-catalog-api-using-the-catalog)=
## Using the Catalog

(discovery-capabilities-api-discovery-catalog-api-rest-url-composition)=
### REST URL Composition

To call a REST resource version, a client MUST join the transport `url` and the version `path`: `url` followed by `path`. For example, `url` `https://example.com/storage` and `path` `/api/v1/fileobject` give `https://example.com/storage/api/v1/fileobject`. When `path` is absent, the client MUST obtain the path from that resource's own API specification and resolve it relative to `url`.

(discovery-capabilities-api-discovery-catalog-api-grpc-selection)=
### gRPC Selection

To call a gRPC service, a client selects the entry whose `grpc.endpoints` contains the fully qualified service name it was generated for, and connects to `grpc.url`. The version is part of the fully qualified name; clients MUST NOT derive a gRPC service name from a REST resource name.

(discovery-capabilities-api-discovery-catalog-api-multiple-matches)=
### Multiple Matches

More than one `DiscoveredService` MAY advertise the same REST resource version or gRPC service. The contract does not rank them. A client MAY select any of them, or apply its own selection policy.

(discovery-capabilities-api-discovery-catalog-api-building-the-catalog)=
## Building the Catalog

This section defines how a discovery service turns one target service's Capabilities API response into one `DiscoveredService`. The input is the list of `ServiceEntry` objects from the target's Capabilities API, plus the REST URL and gRPC URL at which the catalog's audience can reach the target. How a discovery service finds targets and determines their URLs is outside this contract.

(discovery-capabilities-api-discovery-catalog-api-field-mapping)=
### Field Mapping

The Capabilities API uses different field names in its gRPC and REST representations. The mapping rules below use the following correspondence.

| Concept | gRPC field | REST field |
|---|---|---|
| Entry name | `ServiceEntry.service_name` | `ServiceEntry.service_name` |
| gRPC blocks | `ServiceEntry.grpc_services` | `ServiceEntry.grpc` |
| gRPC service name | `GrpcEndpoint.service_name` | `GrpcEndpoint.service` |
| REST blocks | `ServiceEntry.rest_services` | `ServiceEntry.rest` |
| REST resource name | `RestEndpoint.name` | `RestEndpoint.name` |
| Version identifier | `RestVersion.version` | `RestVersion.version` |
| Version path | `RestVersion.url_detail.path` | `RestVersion.detail.path` |

(discovery-capabilities-api-discovery-catalog-api-service-identity)=
### Service Identity

- `name` MUST identify the target service, not an API it implements. It MUST be stable for as long as the target exists, so that clients can correlate catalogs over time.
- `name` is assigned by the discovery service. It is not taken from the Capabilities API `service_name`, because one target can advertise several entries with different `service_name` values.
- `capability_source` MUST be `grpc` when the entry was built from the gRPC representation and `openapi` when it was built from the REST representation.

(discovery-capabilities-api-discovery-catalog-api-grpc-rules)=
### gRPC Rules

1. Collect every non-empty gRPC service name from every gRPC block of every `ServiceEntry`.
2. Remove duplicates.
3. Emit one `GrpcServiceEndpoint` per remaining name.
4. Emit `grpc` only when the target has a gRPC URL for the catalog's audience and at least one service name remains. Otherwise omit `grpc`.

(discovery-capabilities-api-discovery-catalog-api-rest-rules)=
### REST Rules

1. For every REST endpoint in every REST block of every `ServiceEntry`, the resource name is the endpoint `name` or, when that is empty, the `service_name` of its `ServiceEntry`.
2. Discard versions whose identifier is empty.
3. Merge endpoints with the same resource name into one `RestEndpoint`, across all `ServiceEntry` objects. Their versions are combined into one `versions` map.
4. For each version, emit `path` when the reported path is non-empty, and omit it otherwise.
5. When the same resource name and version are reported more than once, the occurrence that appears last in the Capabilities response wins.
6. Omit any `RestEndpoint` whose `versions` map is empty.
7. Emit `rest` only when the target has a REST URL for the catalog's audience and at least one `RestEndpoint` remains. Otherwise omit `rest`.

(discovery-capabilities-api-discovery-catalog-api-audiences)=
### Audiences

A discovery service MAY serve different catalogs to different network audiences, for example one catalog for callers inside a private network and another for public callers. Every URL in a catalog MUST be reachable by the audience that receives it. When a target has no URL for an audience, the corresponding transport block MUST be omitted from that audience's catalog. When neither block remains, the implementation SHOULD omit the `DiscoveredService` from that catalog.

(discovery-capabilities-api-discovery-catalog-api-freshness)=
### Freshness

- The catalog MUST reflect targets that the discovery service currently knows about. A target that no longer exists MUST be removed from the catalog.
- When a known target's Capabilities API is temporarily unreachable, an implementation MAY keep that target's last successfully built entry in the catalog until the target is removed.
- How often an implementation refreshes the catalog is implementation-defined, and SHOULD be consistent with the `Cache-Control` header it sends.

(discovery-capabilities-api-discovery-catalog-api-see-next)=
## See next

- {ref}`Capabilities REST API v1beta <discovery-capabilities-api-rest-api-rest-api-v1beta>`
- {ref}`Capabilities gRPC API v2beta <discovery-capabilities-api-grpc-grpc-api-v2beta>`
