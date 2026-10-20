(auto-discovery-service-service-api-service-specific-http-api)=
# Service-Specific HTTP API

The concrete Auto-Discovery Service exposes HTTP endpoints for aggregated discovery results, version 1 compatibility, authentication bootstrap, and an optional root redirect. These endpoints are specific to this deployment and are separate from the generic Discovery Capabilities API contract that the service queries on target services.

The endpoints are available on both listeners. Port `8080` uses the internal catalog and port `8081` uses the external catalog by default. Compatibility, authentication, and redirect behavior are configured independently of the selected listener.

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this section are to be interpreted as described in [RFC 2119](https://datatracker.ietf.org/doc/html/rfc2119).

(auto-discovery-service-service-api-error-codes)=
## Error Codes

| HTTP status | Endpoint | Condition |
|---|---|---|
| `200 OK` | Service lists and auth config | The requested representation is enabled and available. |
| Configured 3xx status | `/` | Root redirect is enabled; the `Location` header contains the target URL. |
| `404 Not Found` | `/` | Root redirect is disabled. |
| `404 Not Found` | Version 1 service lists | No version 1 endpoint mappings are configured. |
| `404 Not Found` | Auth config | Authentication bootstrap is not configured. |
| `500 Internal Server Error` | Auth config | The configured authentication value is not valid JSON. |

(auto-discovery-service-service-api-response-models)=
## Response Models

(auto-discovery-service-service-api-services-v2-response)=
### ServicesV2Response

| Field | Type | Required | Description |
|---|---|---:|---|
| `schema-version` | integer | Yes | Always `2`. |
| `services` | array of {ref}`DiscoveredService <auto-discovery-service-service-api-discovered-service>` | Yes | Discovered services sorted by name. |

(auto-discovery-service-service-api-discovered-service)=
### DiscoveredService

| Field | Type | Required | Description |
|---|---|---:|---|
| `name` | string | Yes | Kubernetes Service name or configured static endpoint name. |
| `capability_source` | string | Yes | Capability source; currently `grpc`. |
| `rest` | {ref}`RestTransport <auto-discovery-service-service-api-rest-transport>` | No | REST address and advertised resources. Omitted when unresolved or empty. |
| `grpc` | {ref}`GrpcTransport <auto-discovery-service-service-api-grpc-transport>` | No | gRPC address and advertised services. Omitted when unresolved or empty. |

(auto-discovery-service-service-api-rest-transport)=
### RestTransport

| Field | Type | Required | Description |
|---|---|---:|---|
| `url` | string | Yes | Resolved REST base URL. |
| `endpoints` | array of {ref}`RestEndpoint <auto-discovery-service-service-api-rest-endpoint>` | Yes | Named REST resources. |

(auto-discovery-service-service-api-rest-endpoint)=
### RestEndpoint

| Field | Type | Required | Description |
|---|---|---:|---|
| `name` | string | Yes | REST resource name. |
| `versions` | object | Yes | Map from version identifier to {ref}`VersionDetail <auto-discovery-service-service-api-version-detail>`. |

(auto-discovery-service-service-api-version-detail)=
### VersionDetail

| Field | Type | Required | Description |
|---|---|---:|---|
| `path` | string | No | Resource path reported by the target service. |
| `service` | string | No | Reserved service detail when available. |

(auto-discovery-service-service-api-grpc-transport)=
### GrpcTransport

| Field | Type | Required | Description |
|---|---|---:|---|
| `url` | string | Yes | Resolved gRPC base URL. |
| `endpoints` | array of object | Yes | Entries with a required `service` field containing a fully qualified gRPC service name. |

(auto-discovery-service-service-api-services-v1-response)=
### ServicesV1Response

| Field | Type | Required | Description |
|---|---|---:|---|
| `schema-version` | integer | Yes | Always `1`. |
| `services` | array of {ref}`LegacyService <auto-discovery-service-service-api-legacy-service>` | Yes | Services produced from the configured compatibility mappings. |

(auto-discovery-service-service-api-legacy-service)=
### LegacyService

| Field | Type | Required | Description |
|---|---|---:|---|
| `id` | string | Yes | Mapping prefix, with a numeric suffix when several discovered targets match. |
| `name` | string | Yes | Display name from the compatibility mapping. |
| `type` | string | Yes | Legacy service type from the compatibility mapping. |
| `rest` | string | No | Resolved REST base URL. |
| `grpc` | string | No | Resolved gRPC base URL. |

(auto-discovery-service-service-api-get-api-v2beta-services)=
## GET /api/v2beta/services — List Discovered Capabilities

Returns the current catalog snapshot for the listener. The alias `GET /api/v2/services` returns the identical response for clients written before the v2beta path. The response includes `Cache-Control: max-age=30, must-revalidate`. The operation takes no path or query parameters.

**Response:** {ref}`ServicesV2Response <auto-discovery-service-service-api-services-v2-response>`

(auto-discovery-service-service-api-get-api-v2beta-services-example)=
### Example

```console
curl --request GET http://auto-discovery-service:8080/api/v2beta/services
```

```json
{
  "schema-version": 2,
  "services": [
    {
      "name": "storage",
      "capability_source": "grpc",
      "rest": {
        "url": "http://storage.workloads.svc.cluster.local:8080",
        "endpoints": [
          {
            "name": "fileobject",
            "versions": {
              "v1": {
                "path": "/api/v1/fileobject"
              }
            }
          }
        ]
      },
      "grpc": {
        "url": "http://storage.workloads.svc.cluster.local:50051",
        "endpoints": [
          {
            "service": "nvidia.omniverse.storage.fileobject.v1.FileObjectService"
          }
        ]
      }
    }
  ]
}
```

(auto-discovery-service-service-api-get-api-v1-services)=
## GET /api/v1/services — List Legacy Discovery Entries

Returns the version 1 compatibility representation generated from configured capability-to-type mappings. The alias `GET /api/v1alpha/services` returns the same representation. When no mappings are configured, both paths return `404 Not Found`.

The response includes `Cache-Control: max-age=30, must-revalidate`.

**Response:** {ref}`ServicesV1Response <auto-discovery-service-service-api-services-v1-response>`

(auto-discovery-service-service-api-get-api-v1-services-example)=
### Example

```console
curl --request GET http://auto-discovery-service:8080/api/v1/services
```

```json
{
  "schema-version": 1,
  "services": [
    {
      "id": "storage-service",
      "name": "Storage Service",
      "type": "storage",
      "rest": "http://storage.workloads.svc.cluster.local:8080",
      "grpc": "http://storage.workloads.svc.cluster.local:50051"
    }
  ]
}
```

(auto-discovery-service-service-api-get-api-auth-config)=
## GET /api/{version}/auth-config — Read Authentication Bootstrap

Returns the configured authentication bootstrap object. `{version}` is `v1`, `v1alpha`, `v2`, or `v2beta`; all four paths return the same JSON. They return `404 Not Found` when authentication bootstrap is disabled or empty.

The Helm chart produces an object with an OpenID provider metadata URL and named client registrations. It also derives `client_library` and `navigator` registrations from a registration named `default` when those names are not configured explicitly.

(auto-discovery-service-service-api-get-api-auth-config-example)=
### Example

```console
curl --request GET https://discovery.example.com/api/v2beta/auth-config
```

```json
{
  "_comment": "OpenID Authentication",
  "openid_configuration": "https://identity.example.com/.well-known/openid-configuration",
  "clients": {
    "default": {
      "client_id": "discovery-client",
      "scope": "openid profile"
    },
    "client_library": {
      "client_id": "discovery-client",
      "scope": "openid profile"
    },
    "navigator": {
      "client_id": "discovery-client",
      "scope": "openid profile"
    }
  }
}
```

(auto-discovery-service-service-api-get-root)=
## GET / — Redirect to a Configured Destination

When root redirect is enabled, returns the configured redirect status with a `Location` header. When disabled, returns `404 Not Found`. An invalid configured HTTP status falls back to `302 Found`.

(auto-discovery-service-service-api-get-root-example)=
### Example

```console
curl --include https://discovery.example.com/
```

```text
HTTP/1.1 302 Found
location: https://docs.example.com/discovery
```

(auto-discovery-service-service-api-see-next)=
## See next

- {ref}`Compatibility, Authentication, and Ingress <auto-discovery-service-compatibility-and-access-compatibility-authentication-and-ingress>`
