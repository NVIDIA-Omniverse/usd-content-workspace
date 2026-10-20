(discovery-capabilities-api-grpc-grpc-api-v2beta)=
# Capabilities gRPC API v2beta

The `nvidia.omniverse.discovery.capabilities.v2beta` package provides the same capability-reporting function as the REST contract through `CapabilitiesService.ListServices`. The gRPC contract adds request and response pagination fields. REST `v1beta` and gRPC `v2beta` are independently versioned and are not identical wire representations.

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this section are to be interpreted as described in [RFC 2119](https://datatracker.ietf.org/doc/html/rfc2119).

(discovery-capabilities-api-grpc-service-definition)=
## Service Definition

| RPC | Request | Response | Description |
|---|---|---|---|
| `ListServices` | `ListServicesRequest` | `ListServicesResponse` | Returns the public REST resources and gRPC services implemented by the server. |

(discovery-capabilities-api-grpc-pagination)=
## Pagination

`ListServicesRequest.page_size` specifies the maximum number of service entries requested. A server MAY return fewer entries. `ListServicesRequest.page_token` carries a token returned by an earlier call. When another page exists, the response supplies `ListServicesResponse.next_page_token`; an empty token indicates that no later page is available.

A client MUST treat page tokens as opaque and return them unchanged. It SHOULD continue requesting pages until `next_page_token` is empty when it needs the complete capability set.

(discovery-capabilities-api-grpc-error-codes)=
## Error Codes

| gRPC status | REST equivalent | Condition |
|---|---:|---|
| `OK` | `200 OK` | The server returns a capabilities response, including an empty service list when no services are advertised. |
| `INVALID_ARGUMENT` | `400 Bad Request` | The request contains an invalid pagination value. |
| `UNAUTHENTICATED` | `401 Unauthorized` | An implementation that requires caller authentication cannot identify the caller. |
| `PERMISSION_DENIED` | `403 Forbidden` | An authenticated caller is not allowed to list capabilities. |
| `RESOURCE_EXHAUSTED` | `429 Too Many Requests` | The implementation cannot accept the request because a configured resource limit is exhausted. |
| `INTERNAL` | `500 Internal Server Error` | The implementation encounters an unexpected failure. |
| `UNAVAILABLE` | `503 Service Unavailable` | The capabilities service is temporarily unavailable. |

(discovery-capabilities-api-grpc-protocol-buffer-definition)=
## Protocol Buffer Definition

The service and request/response messages are defined as follows:

```protobuf
// Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
//
// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

syntax = "proto3";

package nvidia.omniverse.discovery.capabilities.v2beta;

import "nvidia/omniverse/discovery/capabilities/v2beta/capabilities.proto";

option go_package = "pb.api.nvidia.com/omniverse/discovery/capabilities/v2beta;discovery";
option java_multiple_files = true;
option java_outer_classname = "CapabilitiesServiceProto";
option java_package = "com.nvidia.omniverse.discovery.capabilities.v2beta";

service CapabilitiesService {
  rpc ListServices(ListServicesRequest) returns (ListServicesResponse);
}

message ListServicesRequest {
  int32 page_size = 1;
  string page_token = 2;
}

message ListServicesResponse {
  repeated ServiceEntry services = 1;
  string next_page_token = 2;
}
```

The supporting capability messages are defined as follows:

```protobuf
// Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
//
// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

syntax = "proto3";

package nvidia.omniverse.discovery.capabilities.v2beta;

option go_package = "pb.api.nvidia.com/omniverse/discovery/capabilities/v2beta;discovery";
option java_multiple_files = true;
option java_outer_classname = "CapabilitiesProto";
option java_package = "com.nvidia.omniverse.discovery.capabilities.v2beta";

message ServiceEntry {
  string service_name = 1;
  repeated GrpcService grpc_services = 2;
  repeated RestService rest_services = 3;
}

message GrpcService {
  repeated GrpcEndpoint endpoints = 1;
}

message GrpcEndpoint {
  string service_name = 1;
}

message RestService {
  repeated RestEndpoint endpoints = 1;
}

message RestEndpoint {
  string name = 1;
  repeated RestVersion versions = 2;
}

message RestVersion {
  string version = 1;
  RestVersionDetail url_detail = 2;
}

message RestVersionDetail {
  string path = 1;
}
```

(discovery-capabilities-api-grpc-see-next)=
## See next

- {ref}`References <discovery-capabilities-api-references-references>`
