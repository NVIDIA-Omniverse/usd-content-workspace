(permission-api-grpc-grpc-specification)=
# gRPC Specification

The Permission Service also exposes a gRPC API in the `nvidia.omniverse.permission.v1beta` package. It provides functionality equivalent to the {ref}`REST API <permission-api-rest-api-rest-api>`: the same authorization checks, the same principal/action/resource/context inputs, and the same allow/deny/skip decisions. The full Protocol Buffer definition is embedded at the end of this document.

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD", "SHOULD NOT", "RECOMMENDED",  "MAY", and "OPTIONAL" in this section are to be interpreted as described in [RFC 2119](https://datatracker.ietf.org/doc/html/rfc2119).

(permission-api-grpc-service-definition)=
## Service definition

Service: `nvidia.omniverse.permission.v1beta.PermissionService`.

| RPC | Request | Response | Description |
|-----|---------|----------|-------------|
| `CheckPermission` | `CheckPermissionRequest` | `CheckPermissionResponse` | Checks whether a principal is allowed to perform a single action on a resource. Equivalent to `POST /v1beta/authorization/`. |
| `CheckPermissionBatch` | `CheckPermissionBatchRequest` | `CheckPermissionBatchResponse` | Checks whether a principal is allowed to perform several actions in one request. Equivalent to `POST /v1beta/authorization/batch/`. |

The `decisions` field of `CheckPermissionBatchResponse` contains exactly one `ResourceActionDecisionBatch` per request batch, in order; the `results` within each batch follow the order of the actions in that batch. Each `ResourceActionDecision` MUST set `action` and `service` to echo the request values. `summary` is present only when `condition` is `CONDITION_OR` or `CONDITION_AND`.

(permission-api-grpc-pagination)=
## Pagination

Not applicable. Neither RPC returns a paginated list.

(permission-api-grpc-authentication)=
## Authentication

Clients SHOULD pass authentication via the `authorization` gRPC metadata key in the `Bearer <token>` format. When authentication is enabled, the token MUST be a valid credential accepted by the service (by default a JWT issued by the Identity Provider for a user or a service); missing, invalid, or expired credentials MUST be rejected with `UNAUTHENTICATED`.

An implementation MAY support other authentication mechanisms (for example Basic Auth, API keys, or SAML2) and MAY change how authentication information is passed. When authentication is disabled, the service MAY process requests without credentials.

(permission-api-grpc-error-conditions)=
## Error codes

When a request is accepted for evaluation, the RPC completes with `OK` and the outcome is carried in the response message (`DECISION_ALLOW`, `DECISION_DENY`, or `DECISION_SKIP`). In the REST API the same outcomes appear as HTTP `200` with `"decision": "allow"`, `"deny"`, or `"skip"`. An implicit denial (no matching rule) is `DECISION_DENY` without `reason`, matching REST `deny` with no `reason`.

The following conditions MUST be rejected before evaluation and MUST NOT be returned as a successful RPC whose message carries `DECISION_DENY`. They are reported as standard gRPC statuses and match the client-error cases in the {ref}`REST API error codes <permission-api-rest-api-error-codes>` table (for example a missing required `action` on `CheckPermissionRequest`, or a payload that violates field constraints):

| Condition | gRPC status | REST status |
|-----------|-------------|-------------|
| The request is malformed or violates field constraints (including a missing required `action`). | `INVALID_ARGUMENT` | `400` |
| Authentication credentials are missing, invalid, or expired. | `UNAUTHENTICATED` | `401` |
| The caller is not allowed to check authorization for the specified `principal` (when the implementation rejects the call rather than returning `DECISION_DENY`; see below). | `PERMISSION_DENIED` | `403` |
| The client is rate limited, or the request exceeds the maximum allowed size. | `RESOURCE_EXHAUSTED` | `429` |
| An unexpected server error occurred. | `INTERNAL` | `500` |

When the caller is not allowed to check authorization for the specified `principal`, an implementation SHOULD reject the call with `PERMISSION_DENIED` (REST `403`), as in the table above. An implementation MAY instead evaluate the request and return `DECISION_DENY` in an `OK` response; either way the behavior MUST match its REST API.

(permission-api-grpc-service-discovery-capabilities-service)=
## Service discovery — CapabilitiesService

Because the service is reachable by public clients, an implementation MAY expose the Omniverse discovery `CapabilitiesService` (`nvidia.omniverse.discovery.capabilities.v2alpha.CapabilitiesService`) so discovery clients scanning by the `permission` service type can enumerate the public Permission APIs at runtime. When this service is exposed, `ListServices` MUST be callable without caller authentication, matching discovery-scan expectations.

| RPC | Request | Response | Description |
|-----|---------|----------|-------------|
| `ListServices` | `ListServicesRequest` | `ListServicesResponse` | Returns a single `ServiceEntry` for the `permission` service type, advertising the gRPC service `nvidia.omniverse.permission.v1beta.PermissionService` and the REST endpoint `permission` (version `v1beta`, path `/v1beta/`). |

Only `ListServices` is required when `CapabilitiesService` is implemented; other RPCs defined in the discovery schema are outside the Permission API contract.

(permission-api-grpc-protocol-buffer-definition)=
## Protocol Buffer definition

```protobuf
// SPDX-FileCopyrightText: Copyright 2026 NVIDIA CORPORATION & AFFILIATES
// SPDX-License-Identifier: Apache-2.0
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

syntax = "proto3";
package nvidia.omniverse.permission.v1beta;

import "google/protobuf/struct.proto";

option go_package = "github.com/nvidia/omniverse/permissions/v1beta";
option java_multiple_files = true;
option java_outer_classname = "PermissionServiceProto";
option java_package = "com.nvidia.omniverse.permissions.v1beta";

// Defines APIs for checking authorization rules for other services.
service PermissionService {
  // Checks if the specified principal is allowed
  // to run an operation on the specified object.
  rpc CheckPermission(CheckPermissionRequest) returns (CheckPermissionResponse);

  // Checks if all specified actions are allowed.
  rpc CheckPermissionBatch(CheckPermissionBatchRequest) returns (CheckPermissionBatchResponse);
}

// The message for request for checking if the specified principal (user or service)
// has access to perform a service operation.
message CheckPermissionRequest {
  // The info about a user or a service that tries to perform an operation.
  // Can be omitted if included in the token passed in the Authorization header.
  optional Principal principal = 1;

  // An operation to be performed.
  Action action = 2;

  // An object representing a resource that the principal tries to operate on.
  optional Resource resource = 3;

  // Extra information about the operation,
  // e.g. the ID address and geolocation of the caller.
  optional google.protobuf.Struct context = 4;
}

// Represents the information about the authorized principal.
message Principal {
  // The unique identifier of this principal (the "sub" claim from the token).
  string sub = 1;

  // Other information from the principal token.
  google.protobuf.Struct info = 2;
}

// Represents the information about the authorized operation.
message Action {
  // The operation name.
  string name = 1;

  // The service name where the operation will be performed.
  string service = 2;
}

// Represents the information about a resource being used for authorization
message Resource {
  // Unique identifier of this resource.
  string id = 1;

  // The type used for resource classification.
  string type = 2;

  // Resource information that may be required to evaluate the authorization policy.
  optional google.protobuf.Struct data = 3;
}

// The message with authorization check results.
// Returns the authorization decision and optionally the reason why this decision has been made.
message CheckPermissionResponse {
  // Defines if the request is allowed or denied for the principal
  Decision decision = 1;

  // The message returned for explicit denials.
  // If omitted, then "deny" is implicit - the service could not find any rules for the specified request.
  optional string reason = 2;
}

// Defines an authorization decision that must be taken by the service
enum Decision {
  // Unset value.
  DECISION_UNSPECIFIED = 0;

  // Defines an explicit or implicit "deny" decision.
  // The corresponding reason field can be checked to determine if deny is explicit.
  DECISION_DENY = 1;

  // Defines an explicit "allow" decision.
  DECISION_ALLOW = 2;

  // Defines that action evaluation has been skipped due to a condition match.
  DECISION_SKIP = 3;
}

// The message for making multiple authorization checks in one single request.
// This is a batched version of CheckPermissionRequest message.
message CheckPermissionBatchRequest {
  // Specifies how batches and actions must be evaluated
  optional Condition condition = 1;

  // Defines multiple authorization requests done by CheckPermissionBatch rpc
  repeated CheckPermissionBatch batches = 2;
}

// Defines the condition specifying how batches and actions in CheckPermissionBatchRequest
// must be evaluated.
enum Condition {
  // Evaluates all requests in batches similarly to individual requests.
  // The summary is compiled similarly to "and" condition but does not stop the evaluation
  // after first "deny".
  CONDITION_UNSPECIFIED = 0;

  // Checks if any of the actions is allowed.
  CONDITION_OR = 1;

  // Checks if all specified actions are allowed.
  // Stops after the first "deny" decision.
  CONDITION_AND = 2;
}

// Represents one authorization check done in CheckPermissionBatchRequest.
message CheckPermissionBatch {
  // The info about a user or a service that tries to perform an operation.
  // Can be omitted if included in the token passed in the Authorization header.
  optional Principal principal = 1;

  // Operations checked for the principal against the specified resource.
  repeated Action actions = 2;

  // A JSON object representing a resource that the principal tries to operate on.
  optional Resource resource = 3;

  // Extra information about the operation,
  // e.g. the ID address and geolocation of the caller.
  optional google.protobuf.Struct context = 4;
}

// The message with batched authorization results for CheckPermissionBatchRequest.
message CheckPermissionBatchResponse {
  // The summary decision about all actions in all batches
  // specified in CheckPermissionBatchRequest
  optional CheckPermissionBatchResponseSummary summary = 1;

  // Defines responses for each batch specified in CheckPermissionBatchRequest
  // (the order is preserved).
  repeated ResourceActionDecisionBatch decisions = 2;
}

// The summary for all authorization checks made in CheckPermissionBatchRequest.
// Specified only if `condition` is set in CheckPermissionBatchRequest.
message CheckPermissionBatchResponseSummary {
  // Defines if the request is allowed or denied for all actions specified
  // in CheckPermissionBatchRequest
  Decision decision = 1;

  // The message returned for explicit denials.
  // If omitted, then "deny" is implicit - the service could not find any rules for the specified request.
  optional string reason = 2;
}

// The message that contains results for CheckPermissionBatch.
message ResourceActionDecisionBatch {
  // Represents a decision for each action specified in CheckPermissionBatch.
  repeated ResourceActionDecision results = 1;
}

// The message that contains results for one service action check made in CheckPermissionBatch message.
message ResourceActionDecision {
  // An operation name specified in CheckPermissionBatch.
  string action = 1;

  // The service name specified in `action` for CheckPermissionBatch.
  string service = 2;

  // Defines if the request is allowed or denied for the specified action
  Decision decision = 3;

  // The message returned for explicit denials.
  // If omitted, then "deny" is implicit - the service could not find any rules for the specified request.
  optional string reason = 4;
}
```

(permission-api-grpc-see-next)=
## See next

* {ref}`References <permission-api-references-references>`
