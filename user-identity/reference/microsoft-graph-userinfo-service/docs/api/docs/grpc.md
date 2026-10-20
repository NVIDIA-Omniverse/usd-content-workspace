(userinfo-api-grpc-title)=
# gRPC Specification

The service is defined in the `nvidia.omniverse.userinfo.v1alpha` package and provides equivalent functionality to the REST API using Protocol Buffers.

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this section are to be interpreted as described in [RFC 2119](https://datatracker.ietf.org/doc/html/rfc2119).

(userinfo-api-grpc-service-definition-userinfo-service)=
## Service definition — UserInfoService

| RPC | Request | Response | Description |
|---|---|---|---|
| `GetCurrentUser` | `GetCurrentUserRequest` | `GetCurrentUserResponse` | Returns the profile of the currently authenticated user. The caller's identity is the subject of the bearer token in the `authorization` gRPC metadata key, and the response is the directory record that matches that subject. |
| `ListUsers` | `ListUsersRequest` | `ListUsersResponse` | Lists users in the directory. Supports optional filtering by `email` (exact match) or `display_name` (prefix with `*` suffix, or substring). Results are paginated using `page_size` and `page_token`. |
| `GetUser` | `GetUserRequest` | `GetUserResponse` | Returns a single user by their unique identity provider identifier. Returns `NOT_FOUND` if the user does not exist. |
| `ListGroups` | `ListGroupsRequest` | `ListGroupsResponse` | Lists groups in the directory. Supports optional filtering by `display_name` (prefix or substring). Results are paginated. |
| `GetGroup` | `GetGroupRequest` | `GetGroupResponse` | Returns a single group by its unique identity provider identifier. Returns `NOT_FOUND` if the group does not exist. |
| `ListGroupMembers` | `ListGroupMembersRequest` | `ListGroupMembersResponse` | Lists the users that are direct members of a specific group. Supports optional filtering by `email` (exact match) or `display_name` (prefix or substring). Results are paginated. |
| `ListUserGroups` | `ListUserGroupsRequest` | `ListUserGroupsResponse` | Lists the groups that a specific user is a direct member of. Supports optional display-name filtering and pagination. |
| `GetUserGroup` | `GetUserGroupRequest` | `GetUserGroupResponse` | Checks whether a user is a direct member of a group. Returns the group details if the user is a member. Returns `NOT_FOUND` if the user is not a direct member or if either entity does not exist. |
| `GetGroupMember` | `GetGroupMemberRequest` | `GetGroupMemberResponse` | Checks whether a user is a direct member of a group. Returns the user details if the user is a member. Returns `NOT_FOUND` if the user is not a member or if either entity does not exist. |

(userinfo-api-grpc-pagination)=
## Pagination

List RPCs accept `page_size` (maximum results per page, capped at 100 server-side) and `page_token` (opaque continuation token from a previous response). Responses include `next_page_token`, which is empty when there are no more results.

(userinfo-api-grpc-error-codes)=
## Error codes

| gRPC Status | REST Equivalent | Description |
|---|---|---|
| `INVALID_ARGUMENT` | `400` | The request is malformed, for example an unparseable pagination `page_token`. |
| `UNAUTHENTICATED` | `401` | Missing or invalid bearer token. |
| `PERMISSION_DENIED` | `403` | Insufficient permissions. |
| `NOT_FOUND` | `404` | User or group does not exist, or membership check failed. |
| `RESOURCE_EXHAUSTED` | `429` | Identity provider rate limit exceeded. |
| `UNAVAILABLE` | `502` | Identity provider returned an unexpected error. |
| `INTERNAL` | `500` | Unexpected internal error. |

(userinfo-api-grpc-service-discovery-capabilities-service)=
## Service discovery — CapabilitiesService

An implementation MAY expose the Omniverse discovery `CapabilitiesService` (`nvidia.omniverse.discovery.capabilities.v2alpha.CapabilitiesService`) so discovery clients scanning by the `userinfo` service type can enumerate the public User Info APIs at runtime. When this service is exposed, `ListServices` MUST be callable without caller authentication, matching discovery-scan expectations.

| RPC | Request | Response | Description |
|---|---|---|---|
| `ListServices` | `ListServicesRequest` | `ListServicesResponse` | Returns a single `ServiceEntry` for the `userinfo` service type, advertising the gRPC service `nvidia.omniverse.userinfo.v1alpha.UserInfoService` and the REST endpoint `userinfo` (version `v1alpha`, path `/api/v1alpha/`). |

Only `ListServices` is required when `CapabilitiesService` is implemented; other RPCs defined in the discovery schema are outside the User Info API contract.

(userinfo-api-grpc-protocol-buffer-definition)=
## Protocol Buffer definition

```protobuf
// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
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

package nvidia.omniverse.userinfo.v1alpha;

option go_package = "nvidia.com/omniverse/userinfo/v1alpha;userinfov1alpha";
option java_multiple_files = true;
option java_outer_classname = "UserInfoServiceProto";
option java_package = "com.nvidia.omniverse.userinfo.v1alpha";

/// A user account in the organization's directory.
message User {
  /// Unique identifier of the user in the directory.
  string id = 1;

  /// Full display name of the user (e.g. "Jane Doe").
  string display_name = 2;

  /// First/given name of the user.
  optional string given_name = 3;

  /// Last/family name of the user.
  optional string family_name = 4;

  /// Primary email address of the user.
  optional string email = 5;

  /// Login username (user principal name).
  string username = 6;

  /// Whether the account is enabled in the directory.
  optional bool active = 7;

  /// Job title as recorded in the directory.
  optional string job_title = 8;
}

/// A security or distribution group in the organization's directory.
message Group {
  /// Unique identifier of the group in the directory.
  string id = 1;

  /// Display name of the group.
  string display_name = 2;

  /// Human-readable description of the group's purpose.
  optional string description = 3;
}

/// Read-only identity service providing user and group information
/// from the organization's directory.
service UserInfoService {
  /// Returns the profile of the currently authenticated user. The caller's
  /// identity is determined from the bearer token in the `authorization` metadata.
  rpc GetCurrentUser(GetCurrentUserRequest) returns (GetCurrentUserResponse);

  /// Lists users in the directory with optional filtering by email or display name.
  rpc ListUsers(ListUsersRequest) returns (ListUsersResponse);

  /// Returns a single user by their unique identifier.
  rpc GetUser(GetUserRequest) returns (GetUserResponse);

  /// Lists groups in the directory with optional display-name filtering.
  rpc ListGroups(ListGroupsRequest) returns (ListGroupsResponse);

  /// Returns a single group by its unique identifier.
  rpc GetGroup(GetGroupRequest) returns (GetGroupResponse);

  /// Lists the direct user members of a specific group.
  rpc ListGroupMembers(ListGroupMembersRequest) returns (ListGroupMembersResponse);

  /// Lists the groups that a specific user is a direct member of.
  rpc ListUserGroups(ListUserGroupsRequest) returns (ListUserGroupsResponse);

  /// Verifies direct user membership in a group and returns the group details.
  rpc GetUserGroup(GetUserGroupRequest) returns (GetUserGroupResponse);

  /// Verifies direct user membership in a group and returns the user details.
  rpc GetGroupMember(GetGroupMemberRequest) returns (GetGroupMemberResponse);
}

/// Request for GetCurrentUser. The caller's bearer token is passed
/// via the `authorization` gRPC metadata key.
message GetCurrentUserRequest {}

/// Response for GetCurrentUser.
message GetCurrentUserResponse {
  /// Profile of the authenticated user.
  User user = 1;
}

/// Request for ListUsers.
message ListUsersRequest {
  /// Filter by exact email address.
  optional string email = 1;

  /// Filter by display name. Suffix with `*` for prefix matching (e.g. "Jan*"),
  /// or provide a plain string for substring search.
  optional string display_name = 2;

  /// Maximum number of results per page. Capped at 100 server-side.
  int32 page_size = 3;

  /// Opaque pagination token from a previous ListUsersResponse.
  string page_token = 4;
}

/// Response for ListUsers.
message ListUsersResponse {
  /// Users matching the request filters.
  repeated User users = 1;

  /// Token to retrieve the next page. Empty when there are no more results.
  string next_page_token = 2;
}

/// Request for GetUser.
message GetUserRequest {
  /// Unique identifier of the user to retrieve.
  string user_id = 1;
}

/// Response for GetUser.
message GetUserResponse {
  /// The requested user.
  User user = 1;
}

/// Request for ListGroups.
message ListGroupsRequest {
  /// Filter by display name. Suffix with `*` for prefix matching,
  /// or provide a plain string for substring search.
  optional string display_name = 1;

  /// Maximum number of results per page. Capped at 100 server-side.
  int32 page_size = 2;

  /// Opaque pagination token from a previous ListGroupsResponse.
  string page_token = 3;
}

/// Response for ListGroups.
message ListGroupsResponse {
  /// Groups matching the request filters.
  repeated Group groups = 1;

  /// Token to retrieve the next page. Empty when there are no more results.
  string next_page_token = 2;
}

/// Request for GetGroup.
message GetGroupRequest {
  /// Unique identifier of the group to retrieve.
  string group_id = 1;
}

/// Response for GetGroup.
message GetGroupResponse {
  /// The requested group.
  Group group = 1;
}

/// Request for ListGroupMembers.
message ListGroupMembersRequest {
  /// Unique identifier of the group whose members to list.
  string group_id = 1;

  /// Filter by exact email address.
  optional string email = 2;

  /// Filter by display name. Suffix with `*` for prefix matching (e.g. "Jan*"),
  /// or provide a plain string for substring search.
  optional string display_name = 3;

  /// Maximum number of results per page. Capped at 100 server-side.
  int32 page_size = 4;

  /// Opaque pagination token from a previous ListGroupMembersResponse.
  string page_token = 5;
}

/// Response for ListGroupMembers.
message ListGroupMembersResponse {
  /// Users that are members of the group, matching any filters.
  repeated User users = 1;

  /// Token to retrieve the next page. Empty when there are no more results.
  string next_page_token = 2;
}

/// Request for ListUserGroups.
message ListUserGroupsRequest {
  /// Unique identifier of the user whose groups to list.
  string user_id = 1;

  /// Filter by group display name. Suffix with `*` for prefix matching,
  /// or provide a plain string for substring search.
  optional string display_name = 2;

  /// Maximum number of results per page. Capped at 100 server-side.
  int32 page_size = 3;

  /// Opaque pagination token from a previous ListUserGroupsResponse.
  string page_token = 4;
}

/// Response for ListUserGroups.
message ListUserGroupsResponse {
  /// Groups the user belongs to, matching any filters.
  repeated Group groups = 1;

  /// Token to retrieve the next page. Empty when there are no more results.
  string next_page_token = 2;
}

/// Request for GetUserGroup.
message GetUserGroupRequest {
  /// Unique identifier of the user.
  string user_id = 1;

  /// Unique identifier of the group to check membership for.
  string group_id = 2;
}

/// Response for GetUserGroup.
message GetUserGroupResponse {
  /// The group, when the user is a direct member.
  Group group = 1;
}

/// Request for GetGroupMember.
message GetGroupMemberRequest {
  /// Unique identifier of the group.
  string group_id = 1;

  /// Unique identifier of the user to check membership for.
  string user_id = 2;
}

/// Response for GetGroupMember.
message GetGroupMemberResponse {
  /// The user, when they are a direct member of the group.
  User user = 1;
}
```

(userinfo-api-grpc-see-next)=
## See next

* {ref}`Reference documents <userinfo-api-references-title>`
