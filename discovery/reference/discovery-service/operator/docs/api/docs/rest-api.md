(discovery-capabilities-api-rest-api-rest-api-v1beta)=
# Capabilities REST API v1beta

The REST contract represents discovery capabilities as HTTP and JSON. It defines one successful list operation and a hierarchy of service, transport, endpoint, version, and path models. The source contract does not define an error response body or application-specific error codes.

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this section are to be interpreted as described in [RFC 2119](https://datatracker.ietf.org/doc/html/rfc2119).

(discovery-capabilities-api-rest-api-error-codes)=
## Error Codes

| HTTP status | Error code | Condition |
|---|---|---|
| `200 OK` | — | The server returns the advertised service entries. |

The v1beta OpenAPI contract does not specify non-success statuses or an error model. An implementation MAY return standard HTTP errors, but clients MUST NOT depend on an undocumented response shape.

(discovery-capabilities-api-rest-api-response-models)=
## Response Models

(discovery-capabilities-api-rest-api-list-services-response)=
### ListServicesResponse

| Field | Type | Required | Description |
|---|---|---:|---|
| `services` | array of {ref}`ServiceEntry <discovery-capabilities-api-rest-api-service-entry>` | No | Services and API details implemented by the server. |

(discovery-capabilities-api-rest-api-service-entry)=
### ServiceEntry

| Field | Type | Required | Description |
|---|---|---:|---|
| `service_name` | string | No | Logical service name, such as `fileobject`. |
| `grpc` | array of {ref}`GrpcService <discovery-capabilities-api-rest-api-grpc-service>` | No | gRPC transport blocks reported for the service. |
| `rest` | array of {ref}`RestService <discovery-capabilities-api-rest-api-rest-service>` | No | REST transport blocks reported for the service. |

(discovery-capabilities-api-rest-api-grpc-service)=
### GrpcService

| Field | Type | Required | Description |
|---|---|---:|---|
| `endpoints` | array of {ref}`GrpcEndpoint <discovery-capabilities-api-rest-api-grpc-endpoint>` | No | Fully qualified gRPC services exposed by the server. |

(discovery-capabilities-api-rest-api-grpc-endpoint)=
### GrpcEndpoint

| Field | Type | Required | Description |
|---|---|---:|---|
| `service` | string | No | Fully qualified Protocol Buffer service name. |

(discovery-capabilities-api-rest-api-rest-service)=
### RestService

| Field | Type | Required | Description |
|---|---|---:|---|
| `endpoints` | array of {ref}`RestEndpoint <discovery-capabilities-api-rest-api-rest-endpoint>` | No | Named REST resources exposed by the server. |

(discovery-capabilities-api-rest-api-rest-endpoint)=
### RestEndpoint

| Field | Type | Required | Description |
|---|---|---:|---|
| `name` | string | No | Logical REST resource name, such as `fileobject`. |
| `versions` | array of {ref}`RestVersion <discovery-capabilities-api-rest-api-rest-version>` | No | Supported versions of the resource. |

(discovery-capabilities-api-rest-api-rest-version)=
### RestVersion

| Field | Type | Required | Description |
|---|---|---:|---|
| `version` | string | Yes | Version identifier, such as `v1` or `v1alpha`. |
| `detail` | {ref}`RestVersionDetail <discovery-capabilities-api-rest-api-rest-version-detail>` | No | Version-specific REST location detail. |

(discovery-capabilities-api-rest-api-rest-version-detail)=
### RestVersionDetail

| Field | Type | Required | Description |
|---|---|---:|---|
| `path` | string | No | Path for the resource version, such as `/api/v1/fileobject`. |

(discovery-capabilities-api-rest-api-get-v1beta-services)=
## GET /v1beta/services — List Implemented Service APIs

Returns the services and transport-specific endpoints implemented by the responding server. The operation takes no path or query parameters.

**Response:** {ref}`ListServicesResponse <discovery-capabilities-api-rest-api-list-services-response>`

(discovery-capabilities-api-rest-api-get-v1beta-services-example)=
### Example

```console
curl --request GET \
  --header 'Accept: application/json' \
  https://service.example.com/v1beta/services
```

```json
{
  "services": [
    {
      "service_name": "fileobject",
      "grpc": [
        {
          "endpoints": [
            {
              "service": "nvidia.omniverse.storage.fileobject.v1.FileObjectService"
            }
          ]
        }
      ],
      "rest": [
        {
          "endpoints": [
            {
              "name": "fileobject",
              "versions": [
                {
                  "version": "v1",
                  "detail": {
                    "path": "/api/v1/fileobject"
                  }
                }
              ]
            }
          ]
        }
      ]
    }
  ]
}
```

(discovery-capabilities-api-rest-api-see-next)=
## See next

- {ref}`Capabilities gRPC API v2beta <discovery-capabilities-api-grpc-grpc-api-v2beta>`
