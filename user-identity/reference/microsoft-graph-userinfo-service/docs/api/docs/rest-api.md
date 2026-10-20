(userinfo-api-rest-api-title)=
# REST API

User Info Service exposes a REST (HTTP/JSON) API that provides read-only access to user and group information from the organization's directory.

All endpoints return JSON responses. All error responses use a consistent body with `error` (machine-readable code) and `message` (human-readable description) fields.

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this section are to be interpreted as described in [RFC 2119](https://datatracker.ietf.org/doc/html/rfc2119).

(userinfo-api-rest-api-error-codes)=
## Error codes

| Status Code | Error Code | Condition |
|---|---|---|
| `400 Bad Request` | `bad_request` | The request is malformed, for example an unparseable pagination `cursor`. |
| `401 Unauthorized` | `unauthorized` | Missing or invalid bearer token. |
| `403 Forbidden` | `forbidden` | The authenticated user does not have permission to perform this operation. |
| `404 Not Found` | `not_found` | The requested user or group does not exist, or membership check failed. |
| `429 Too Many Requests` | `rate_limited` | The identity provider's rate limit has been exceeded. The `Retry-After` header indicates when to retry. |
| `502 Bad Gateway` | `bad_gateway` | The identity provider returned an unexpected error. |
| `500 Internal Server Error` | `internal_error` | An unexpected internal error occurred. |

(userinfo-api-rest-api-response-models)=
## Response models

(userinfo-api-rest-api-user-object)=

**User** — a user account in the organization's directory.

| Field | Type | Required | Description |
|---|---|---|---|
| `id` | string | yes | Unique identifier of the user in the identity provider. |
| `display_name` | string | yes | Full display name (e.g. "Jane Doe"). |
| `given_name` | string | no | First/given name. |
| `family_name` | string | no | Last/family name. |
| `email` | string | no | Primary email address. |
| `username` | string | yes | Login username (user principal name). |
| `active` | boolean | no | Whether the account is enabled in the directory. |
| `job_title` | string | no | Job title as recorded in the directory. |

(userinfo-api-rest-api-group-object)=

**Group** — a security or distribution group in the organization's directory.

| Field | Type | Required | Description |
|---|---|---|---|
| `id` | string | yes | Unique identifier of the group in the identity provider. |
| `display_name` | string | yes | Display name of the group. |
| `description` | string | no | Human-readable description of the group's purpose. |

---

(userinfo-api-rest-api-get-users-me)=
## GET /api/v1alpha/users/me — Get current user

Returns the profile of the currently authenticated user. The caller's identity is the subject of the bearer token in the `Authorization: Bearer <token>` header, and the response is the directory record that matches that subject. The response reflects the same directory data as all other endpoints.

A typical use case is displaying the current user's name and email in an application header or profile menu after sign-in.

**Parameters:** None.

**Response:** `200 OK` — {ref}`User <userinfo-api-rest-api-user-object>` object.

**Example:**

Request:

```http
GET /api/v1alpha/users/me HTTP/1.1
Authorization: Bearer eyJhbG...
```

Response:

```http
HTTP/1.1 200 OK
Content-Type: application/json
```

```json
{
  "id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "display_name": "Jane Doe",
  "given_name": "Jane",
  "family_name": "Doe",
  "email": "jane.doe@example.com",
  "username": "jane.doe@example.com",
  "active": true,
  "job_title": "Senior Engineer"
}
```

---

(userinfo-api-rest-api-get-users)=
## GET /api/v1alpha/users — List users

Lists users in the directory with optional filtering. Results are paginated.

This endpoint supports two primary usage patterns. First, it can resolve a known email address to a user profile — for example, during migration from email-based Nucleus ACLs to identifier-based policies, the migration tool queries by `email` to obtain the corresponding stable `id`. Second, it can search by display name to power auto-completion in user interfaces — an administrator types a partial name and the UI presents matching results for selection.

**Query parameters:**

| Parameter | Required | Description |
|---|---|---|
| `email` | no | Filter by exact email address. |
| `display_name` | no | Filter by display name. Append `*` for prefix matching (e.g. `Jan*`). Without `*`, performs substring search. |
| `limit` | no | Maximum number of results per page. Default: 20. Maximum: 100. |
| `cursor` | no | Opaque pagination token from a previous response's `next_cursor` field. Omit for the first page. |

**Response:** `200 OK` — Paginated list of {ref}`User <userinfo-api-rest-api-user-object>` objects.

| Field | Type | Description |
|---|---|---|
| `items` | array of {ref}`User <userinfo-api-rest-api-user-object>` | Users matching the request filters. |
| `next_cursor` | string or null | Opaque token to retrieve the next page. Null when there are no more results. |

**Example — resolve an email to a user ID:**

Request:

```http
GET /api/v1alpha/users?email=jane.doe@example.com HTTP/1.1
Authorization: Bearer eyJhbG...
```

Response:

```http
HTTP/1.1 200 OK
Content-Type: application/json
```

```json
{
  "items": [
    {
      "id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
      "display_name": "Jane Doe",
      "email": "jane.doe@example.com",
      "username": "jane.doe@example.com"
    }
  ],
  "next_cursor": null
}
```

**Example — auto-complete by display name prefix:**

Request:

```http
GET /api/v1alpha/users?display_name=Jan*&limit=5 HTTP/1.1
Authorization: Bearer eyJhbG...
```

Response:

```http
HTTP/1.1 200 OK
Content-Type: application/json
```

```json
{
  "items": [
    {
      "id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
      "display_name": "Jane Doe",
      "email": "jane.doe@example.com",
      "username": "jane.doe@example.com"
    },
    {
      "id": "f9e8d7c6-b5a4-3210-fedc-ba0987654321",
      "display_name": "Jan Smith",
      "email": "jan.smith@example.com",
      "username": "jan.smith@example.com"
    }
  ],
  "next_cursor": null
}
```

---

(userinfo-api-rest-api-get-user-by-id)=
## GET /api/v1alpha/users/{userId} — Get user by ID

Returns a single user by their unique identity provider identifier.

This endpoint is used when a service or UI already has a user identifier — for example, from an authorization policy — and needs to retrieve the corresponding human-readable profile (display name, email, job title) for presentation.

**Path parameters:**

| Parameter | Description |
|---|---|
| `userId` | Unique identifier of the user. |

**Response:** `200 OK` — {ref}`User <userinfo-api-rest-api-user-object>` object.

**Example:**

Request:

```http
GET /api/v1alpha/users/a1b2c3d4-e5f6-7890-abcd-ef1234567890 HTTP/1.1
Authorization: Bearer eyJhbG...
```

Response:

```http
HTTP/1.1 200 OK
Content-Type: application/json
```

```json
{
  "id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "display_name": "Jane Doe",
  "email": "jane.doe@example.com",
  "username": "jane.doe@example.com",
  "active": true,
  "job_title": "Senior Engineer"
}
```

---

(userinfo-api-rest-api-list-user-groups)=
## GET /api/v1alpha/users/{userId}/groups — List groups for a user

Lists the groups that the specified user is a direct member of. Supports optional display-name filtering. Results are paginated.

This endpoint is used to determine what groups a user directly belongs to — for example, to display a user's group memberships in an administration panel, or to evaluate group-based access control decisions.

**Path parameters:**

| Parameter | Description |
|---|---|
| `userId` | Unique identifier of the user. |

**Query parameters:**

| Parameter | Required | Description |
|---|---|---|
| `display_name` | no | Filter by group display name. Append `*` for prefix matching, otherwise substring search. |
| `limit` | no | Maximum number of results per page. Default: 20. Maximum: 100. |
| `cursor` | no | Opaque pagination token from a previous response. Omit for the first page. |

**Response:** `200 OK` — Paginated list of {ref}`Group <userinfo-api-rest-api-group-object>` objects.

| Field | Type | Description |
|---|---|---|
| `items` | array of {ref}`Group <userinfo-api-rest-api-group-object>` | Groups the user is a direct member of, matching any filters. |
| `next_cursor` | string or null | Opaque token to retrieve the next page. Null when there are no more results. |

**Example:**

Request:

```http
GET /api/v1alpha/users/a1b2c3d4-e5f6-7890-abcd-ef1234567890/groups HTTP/1.1
Authorization: Bearer eyJhbG...
```

Response:

```http
HTTP/1.1 200 OK
Content-Type: application/json
```

```json
{
  "items": [
    {
      "id": "g1h2i3j4-k5l6-7890-mnop-qr1234567890",
      "display_name": "Engineering",
      "description": "All engineering staff"
    },
    {
      "id": "s1t2u3v4-w5x6-7890-yzab-cd1234567890",
      "display_name": "Project Alpha",
      "description": "Project Alpha team members"
    }
  ],
  "next_cursor": null
}
```

---

(userinfo-api-rest-api-get-user-group)=
## GET /api/v1alpha/users/{userId}/groups/{groupId} — Check user group membership

Checks whether the specified user is a direct member of the specified group. If the user is a member, returns the group details. Returns `404 Not Found` if the user is not a direct member of the group or if either the user or group does not exist.

This endpoint is useful for point membership checks — for example, verifying that a user directly belongs to an "Administrators" group before granting elevated access, without fetching the user's entire group list.

**Path parameters:**

| Parameter | Description |
|---|---|
| `userId` | Unique identifier of the user. |
| `groupId` | Unique identifier of the group. |

**Response:** `200 OK` — {ref}`Group <userinfo-api-rest-api-group-object>` object (confirming membership).

**Example:**

Request:

```http
GET /api/v1alpha/users/a1b2c3d4-e5f6-7890-abcd-ef1234567890/groups/g1h2i3j4-k5l6-7890-mnop-qr1234567890 HTTP/1.1
Authorization: Bearer eyJhbG...
```

Response:

```http
HTTP/1.1 200 OK
Content-Type: application/json
```

```json
{
  "id": "g1h2i3j4-k5l6-7890-mnop-qr1234567890",
  "display_name": "Engineering",
  "description": "All engineering staff"
}
```

---

(userinfo-api-rest-api-get-groups)=
## GET /api/v1alpha/groups — List groups

Lists groups in the directory with optional display-name filtering. Results are paginated.

This endpoint is used for group discovery and auto-completion — for example, when an administrator is configuring an access policy on an Omniverse Storage directory and begins typing a group name to grant permissions to.

**Query parameters:**

| Parameter | Required | Description |
|---|---|---|
| `display_name` | no | Filter by group display name. Append `*` for prefix matching, otherwise substring search. |
| `limit` | no | Maximum number of results per page. Default: 20. Maximum: 100. |
| `cursor` | no | Opaque pagination token from a previous response. Omit for the first page. |

**Response:** `200 OK` — Paginated list of {ref}`Group <userinfo-api-rest-api-group-object>` objects.

| Field | Type | Description |
|---|---|---|
| `items` | array of {ref}`Group <userinfo-api-rest-api-group-object>` | Groups matching the request filters. |
| `next_cursor` | string or null | Opaque token to retrieve the next page. Null when there are no more results. |

**Example — search for groups starting with "Eng":**

Request:

```http
GET /api/v1alpha/groups?display_name=Eng*&limit=5 HTTP/1.1
Authorization: Bearer eyJhbG...
```

Response:

```http
HTTP/1.1 200 OK
Content-Type: application/json
```

```json
{
  "items": [
    {
      "id": "g1h2i3j4-k5l6-7890-mnop-qr1234567890",
      "display_name": "Engineering",
      "description": "All engineering staff"
    },
    {
      "id": "e2f3g4h5-i6j7-8901-klmn-op2345678901",
      "display_name": "Engineering Leads",
      "description": "Engineering team leads"
    }
  ],
  "next_cursor": null
}
```

---

(userinfo-api-rest-api-get-group-by-id)=
## GET /api/v1alpha/groups/{groupId} — Get group by ID

Returns a single group by its unique identity provider identifier.

This endpoint is used when a service or UI has a group identifier — for example, from an authorization policy entry — and needs to display the group's name and description to an administrator.

**Path parameters:**

| Parameter | Description |
|---|---|
| `groupId` | Unique identifier of the group. |

**Response:** `200 OK` — {ref}`Group <userinfo-api-rest-api-group-object>` object.

**Example:**

Request:

```http
GET /api/v1alpha/groups/g1h2i3j4-k5l6-7890-mnop-qr1234567890 HTTP/1.1
Authorization: Bearer eyJhbG...
```

Response:

```http
HTTP/1.1 200 OK
Content-Type: application/json
```

```json
{
  "id": "g1h2i3j4-k5l6-7890-mnop-qr1234567890",
  "display_name": "Engineering",
  "description": "All engineering staff"
}
```

---

(userinfo-api-rest-api-list-group-members)=
## GET /api/v1alpha/groups/{groupId}/users — List group members

Lists the users that are direct members of the specified group. Supports optional email and display-name filtering. Results are paginated.

This endpoint is the inverse of listing a user's groups — it answers "who belongs to this group?" rather than "what groups does this user belong to?". A typical use case is displaying the member list of a group in an administration panel or verifying which users have been granted access through a group-based policy.

**Path parameters:**

| Parameter | Description |
|---|---|
| `groupId` | Unique identifier of the group. |

**Query parameters:**

| Parameter | Required | Description |
|---|---|---|
| `email` | no | Filter by exact email address. |
| `display_name` | no | Filter by display name. Append `*` for prefix matching (e.g. `Jan*`). Without `*`, performs substring search. |
| `limit` | no | Maximum number of results per page. Default: 20. Maximum: 100. |
| `cursor` | no | Opaque pagination token from a previous response's `next_cursor` field. Omit for the first page. |

**Response:** `200 OK` — Paginated list of {ref}`User <userinfo-api-rest-api-user-object>` objects.

| Field | Type | Description |
|---|---|---|
| `items` | array of {ref}`User <userinfo-api-rest-api-user-object>` | Users that are members of the group, matching any filters. |
| `next_cursor` | string or null | Opaque token to retrieve the next page. Null when there are no more results. |

**Example:**

Request:

```http
GET /api/v1alpha/groups/g1h2i3j4-k5l6-7890-mnop-qr1234567890/users HTTP/1.1
Authorization: Bearer eyJhbG...
```

Response:

```http
HTTP/1.1 200 OK
Content-Type: application/json
```

```json
{
  "items": [
    {
      "id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
      "display_name": "Jane Doe",
      "email": "jane.doe@example.com",
      "username": "jane.doe@example.com",
      "active": true,
      "job_title": "Senior Engineer"
    },
    {
      "id": "f9e8d7c6-b5a4-3210-fedc-ba0987654321",
      "display_name": "Jan Smith",
      "email": "jan.smith@example.com",
      "username": "jan.smith@example.com",
      "active": true,
      "job_title": "Staff Engineer"
    }
  ],
  "next_cursor": null
}
```

---

(userinfo-api-rest-api-get-group-member)=
## GET /api/v1alpha/groups/{groupId}/users/{userId} — Get group member

Checks whether the specified user is a direct member of the specified group. If the user is a member, returns the user details. Returns `404 Not Found` if the user is not a direct member of the group or if either the user or group does not exist.

This endpoint is the inverse of `GET /api/v1alpha/users/{userId}/groups/{groupId}` — it answers "is this user in the group?" from the group's perspective and returns the user object rather than the group object.

**Path parameters:**

| Parameter | Description |
|---|---|
| `groupId` | Unique identifier of the group. |
| `userId` | Unique identifier of the user. |

**Response:** `200 OK` — {ref}`User <userinfo-api-rest-api-user-object>` object.

**Example:**

Request:

```http
GET /api/v1alpha/groups/g1h2i3j4-k5l6-7890-mnop-qr1234567890/users/a1b2c3d4-e5f6-7890-abcd-ef1234567890 HTTP/1.1
Authorization: Bearer eyJhbG...
```

Response:

```http
HTTP/1.1 200 OK
Content-Type: application/json
```

```json
{
  "id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "display_name": "Jane Doe",
  "email": "jane.doe@example.com",
  "username": "jane.doe@example.com",
  "active": true,
  "job_title": "Senior Engineer"
}
```

(userinfo-api-rest-api-openapi)=
## OpenAPI 3.0 Specification

```yaml
openapi: 3.0.3
info:
  title: User Info Service
  description: >-
    Read-only identity service providing user and group information
    from the organization's directory for the NVIDIA Omniverse platform.
  version: v1alpha
  contact:
    name: NVIDIA Omniverse
paths:
  /api/v1alpha/users/me:
    get:
      summary: Get current user
      description: >-
        Returns the profile of the currently authenticated user.
        The caller's identity is determined from the bearer token
        subject, and the response is the matching directory record.
      operationId: getCurrentUser
      tags: [Users]
      security:
        - bearerAuth: []
      responses:
        "200":
          description: The current user's profile.
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/User"
        "401":
          $ref: "#/components/responses/Unauthorized"

  /api/v1alpha/users:
    get:
      summary: List users
      description: >-
        Lists users in the directory with optional filtering
        by email or display name.
      operationId: listUsers
      tags: [Users]
      security:
        - bearerAuth: []
      parameters:
        - $ref: "#/components/parameters/UserEmail"
        - $ref: "#/components/parameters/DisplayName"
        - $ref: "#/components/parameters/Limit"
        - $ref: "#/components/parameters/Cursor"
      responses:
        "200":
          description: A paginated list of users.
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/UserPage"
        "401":
          $ref: "#/components/responses/Unauthorized"

  /api/v1alpha/users/{userId}:
    get:
      summary: Get user by ID
      description: Returns a single user by their unique identity provider identifier.
      operationId: getUser
      tags: [Users]
      security:
        - bearerAuth: []
      parameters:
        - $ref: "#/components/parameters/UserId"
      responses:
        "200":
          description: The requested user.
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/User"
        "401":
          $ref: "#/components/responses/Unauthorized"
        "404":
          $ref: "#/components/responses/NotFound"

  /api/v1alpha/users/{userId}/groups:
    get:
      summary: List groups for a user
      description: Lists the groups that the specified user is a direct member of.
      operationId: listUserGroups
      tags: [Users]
      security:
        - bearerAuth: []
      parameters:
        - $ref: "#/components/parameters/UserId"
        - $ref: "#/components/parameters/DisplayName"
        - $ref: "#/components/parameters/Limit"
        - $ref: "#/components/parameters/Cursor"
      responses:
        "200":
          description: A paginated list of groups the user belongs to.
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/GroupPage"
        "401":
          $ref: "#/components/responses/Unauthorized"
        "404":
          $ref: "#/components/responses/NotFound"

  /api/v1alpha/users/{userId}/groups/{groupId}:
    get:
      summary: Check user group membership
      description: >-
        Checks whether the specified user is a direct member of the specified group.
        Returns the group details if the user is a member.
      operationId: getUserGroup
      tags: [Users]
      security:
        - bearerAuth: []
      parameters:
        - $ref: "#/components/parameters/UserId"
        - $ref: "#/components/parameters/GroupId"
      responses:
        "200":
          description: The user is a member of the group.
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/Group"
        "401":
          $ref: "#/components/responses/Unauthorized"
        "404":
          $ref: "#/components/responses/NotFound"

  /api/v1alpha/groups:
    get:
      summary: List groups
      description: Lists groups in the directory with optional display-name filtering.
      operationId: listGroups
      tags: [Groups]
      security:
        - bearerAuth: []
      parameters:
        - $ref: "#/components/parameters/DisplayName"
        - $ref: "#/components/parameters/Limit"
        - $ref: "#/components/parameters/Cursor"
      responses:
        "200":
          description: A paginated list of groups.
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/GroupPage"
        "401":
          $ref: "#/components/responses/Unauthorized"

  /api/v1alpha/groups/{groupId}:
    get:
      summary: Get group by ID
      description: Returns a single group by its unique identity provider identifier.
      operationId: getGroup
      tags: [Groups]
      security:
        - bearerAuth: []
      parameters:
        - $ref: "#/components/parameters/GroupId"
      responses:
        "200":
          description: The requested group.
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/Group"
        "401":
          $ref: "#/components/responses/Unauthorized"
        "404":
          $ref: "#/components/responses/NotFound"

  /api/v1alpha/groups/{groupId}/users:
    get:
      summary: List group members
      description: Lists the users that are direct members of the specified group.
      operationId: listGroupMembers
      tags: [Groups]
      security:
        - bearerAuth: []
      parameters:
        - $ref: "#/components/parameters/GroupId"
        - $ref: "#/components/parameters/UserEmail"
        - $ref: "#/components/parameters/DisplayName"
        - $ref: "#/components/parameters/Limit"
        - $ref: "#/components/parameters/Cursor"
      responses:
        "200":
          description: A paginated list of users that are members of the group.
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/UserPage"
        "401":
          $ref: "#/components/responses/Unauthorized"
        "404":
          $ref: "#/components/responses/NotFound"

  /api/v1alpha/groups/{groupId}/users/{userId}:
    get:
      summary: Get group member
      description: >-
        Checks whether the specified user is a direct member of the group.
        Returns the user details if the user is a member.
      operationId: getGroupMember
      tags: [Groups]
      security:
        - bearerAuth: []
      parameters:
        - $ref: "#/components/parameters/GroupId"
        - $ref: "#/components/parameters/UserId"
      responses:
        "200":
          description: The user is a member of the group.
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/User"
        "401":
          $ref: "#/components/responses/Unauthorized"
        "404":
          $ref: "#/components/responses/NotFound"

components:
  securitySchemes:
    bearerAuth:
      type: http
      scheme: bearer
      bearerFormat: JWT

  parameters:
    UserId:
      name: userId
      in: path
      required: true
      description: Unique identifier of the user.
      schema:
        type: string
    GroupId:
      name: groupId
      in: path
      required: true
      description: Unique identifier of the group.
      schema:
        type: string
    UserEmail:
      name: email
      in: query
      required: false
      description: Filter by exact email address.
      schema:
        type: string
    DisplayName:
      name: display_name
      in: query
      required: false
      description: >-
        Filter by display name. Append * for prefix matching (e.g. "Jan*"),
        otherwise performs substring search.
      schema:
        type: string
    Limit:
      name: limit
      in: query
      required: false
      description: Maximum number of results per page (default 20, max 100).
      schema:
        type: integer
        default: 20
        minimum: 1
        maximum: 100
    Cursor:
      name: cursor
      in: query
      required: false
      description: Opaque pagination token from a previous response.
      schema:
        type: string

  schemas:
    User:
      type: object
      required: [id, display_name, username]
      properties:
        id:
          type: string
          description: Unique identifier of the user in the identity provider.
        display_name:
          type: string
          description: Full display name (e.g. "Jane Doe").
        given_name:
          type: string
          description: First/given name.
        family_name:
          type: string
          description: Last/family name.
        email:
          type: string
          description: Primary email address.
        username:
          type: string
          description: Login username (user principal name).
        active:
          type: boolean
          description: Whether the account is enabled in the directory.
        job_title:
          type: string
          description: Job title as recorded in the directory.

    Group:
      type: object
      required: [id, display_name]
      properties:
        id:
          type: string
          description: Unique identifier of the group in the identity provider.
        display_name:
          type: string
          description: Display name of the group.
        description:
          type: string
          description: Human-readable description of the group's purpose.

    UserPage:
      type: object
      required: [items]
      properties:
        items:
          type: array
          items:
            $ref: "#/components/schemas/User"
        next_cursor:
          type: string
          nullable: true
          description: >-
            Opaque token to retrieve the next page.
            Null when there are no more results.

    GroupPage:
      type: object
      required: [items]
      properties:
        items:
          type: array
          items:
            $ref: "#/components/schemas/Group"
        next_cursor:
          type: string
          nullable: true
          description: >-
            Opaque token to retrieve the next page.
            Null when there are no more results.

    Error:
      type: object
      required: [error, message]
      properties:
        error:
          type: string
          description: Machine-readable error code.
        message:
          type: string
          description: Human-readable error description.

  responses:
    Unauthorized:
      description: Missing or invalid bearer token.
      content:
        application/json:
          schema:
            $ref: "#/components/schemas/Error"
    NotFound:
      description: The requested resource does not exist.
      content:
        application/json:
          schema:
            $ref: "#/components/schemas/Error"
```

(userinfo-api-rest-api-see-next)=
## See next

* {ref}`gRPC specification <userinfo-api-grpc-title>`
