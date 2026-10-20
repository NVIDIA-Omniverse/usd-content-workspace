(permission-service-diagnostics-diagnostics-api)=
# Diagnostics API

Permission Service serves a diagnostics API that reports **which policies would be evaluated** for a given authorization request, and the evaluation priority that would apply, instead of returning a decision. It is a troubleshooting tool for policy authors: when a request is allowed or denied unexpectedly, diagnostics shows the exact candidate set and the order it would be evaluated in, without changing any state.

The API is specific to this service and is not part of the published Omniverse Storage Permission API, so it is documented in full here. It is always available — there is no switch that turns it off — and it reuses the request bodies of the authorization API, which the Omniverse Storage Permission API documentation specifies.

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this document are to be interpreted as described in [RFC 2119](https://datatracker.ietf.org/doc/html/rfc2119).

(permission-service-diagnostics-endpoints)=
## Endpoints

| Method | Path                                   | Purpose                                                                              |
|--------|----------------------------------------|--------------------------------------------------------------------------------------|
| POST   | `/v1beta/diagnostics/authorize/`       | Report the candidate policies and evaluation priority for one authorization request. |
| POST   | `/v1beta/diagnostics/authorize/batch/` | Report the same per action, for a batched authorization request.                     |

(permission-service-diagnostics-authentication-and-authorization)=
## Authentication and authorization

When authentication is enabled, the caller MUST present a valid bearer token issued by the configured identity provider (a user or service token):

```text
Authorization: Bearer <accessToken>
```

The caller MUST also be authorized for `Action::"permissions:diagnostics"`. Unlike the authorization API, this check is performed **even when the request names the caller's own principal**, because the response exposes stored policies rather than a single decision. The check runs before any policy is read, and a caller without the grant receives `403 Forbidden`.

For how the grant is seeded and scoped, see {ref}`Granting administrative permissions <permission-service-deployment-granting-administrative-permissions>` and {ref}`Meta-permissions <permission-service-policy-language-meta-permissions>`.

(permission-service-diagnostics-error-codes)=
## Error codes

| Status                      | Condition                                                                                  |
|-----------------------------|--------------------------------------------------------------------------------------------|
| `400 Bad Request`           | The payload parsed, but the request is invalid — for example, a resource id that cannot be encoded. |
| `401 Unauthorized`          | The bearer token is missing, invalid, or expired (authentication enabled).                  |
| `403 Forbidden`             | The caller is not authorized for `Action::"permissions:diagnostics"`.                       |
| `422 Unprocessable Entity`  | The request payload failed validation — a missing `action`, or a field that violates its length or pattern constraint. |
| `500 Internal Server Error` | An unexpected error occurred while reading metadata or policies.                            |

Error bodies are a plain JSON string, as specified for the authorization API in the Omniverse Storage Permission API documentation.

(permission-service-diagnostics-request-and-response-models)=
## Request and response models

Both endpoints accept the authorization request bodies — `AuthorizationPayload` for the single endpoint and `AuthorizationBatchRequest` for the batched one. Their fields are specified in the Omniverse Storage Permission API documentation and are not repeated here. The response models below are specific to this API.

(permission-service-diagnostics-diagnosticsauthorizeresponse)=
### DiagnosticsAuthorizeResponse

Response of `POST /v1beta/diagnostics/authorize/`.

| Field                 | Type                              | Required | Description                                                                                         |
|-----------------------|-----------------------------------|----------|-----------------------------------------------------------------------------------------------------|
| `evaluation_priority` | string                            | yes      | Evaluation priority that would apply: `"forbid"` (default) or `"permit"`. Resolved from the resource type registered in metadata. |
| `policies`            | array of {ref}`DiagnosticsPolicy <permission-service-diagnostics-diagnosticspolicy>` | yes | Candidate policies, in the order they would be evaluated.                    |

(permission-service-diagnostics-diagnosticspolicy)=
### DiagnosticsPolicy

One candidate policy. The scope fields are present only when the policy pins that dimension.

| Field       | Type    | Required | Description                                                                                  |
|-------------|---------|----------|----------------------------------------------------------------------------------------------|
| `id`        | integer | yes      | Policy identifier.                                                                           |
| `order`     | integer | yes      | Evaluation order of the policy. Lower values are evaluated first.                             |
| `policy`    | string  | yes      | Cedar policy text.                                                                           |
| `principal` | object  | no       | Principal scope, as `{ "sub": "<id>" }`.                                                     |
| `group`     | object  | no       | Group scope of a `principal in Group::"<id>"` policy, as `{ "id": "<id>" }`.                 |
| `action`    | object  | no       | Action scope, as `{ "name": "<name>", "service": "<service>" }`.                             |
| `resource`  | object  | no       | Resource scope, as `{ "id": "<id>", "type": "<type>", "data": {} }`.                          |

(permission-service-diagnostics-diagnosticsauthorizebatchresponse)=
### DiagnosticsAuthorizeBatchResponse

Response of `POST /v1beta/diagnostics/authorize/batch/`.

| Field     | Type                                 | Required | Description                                                          |
|-----------|--------------------------------------|----------|----------------------------------------------------------------------|
| `results` | array of {ref}`DiagnosticsBatchResult <permission-service-diagnostics-diagnosticsbatchresult>` | yes | One entry per batch in the request, in request order. |

(permission-service-diagnostics-diagnosticsbatchresult)=
### DiagnosticsBatchResult

Diagnostics for one batch entry.

| Field                 | Type                                    | Required | Description                                                                    |
|-----------------------|-----------------------------------------|----------|--------------------------------------------------------------------------------|
| `resource`            | object                                  | no       | Resource the batch entry targets, as submitted. Absent when the entry has no resource. |
| `evaluation_priority` | string                                  | yes      | Evaluation priority for this entry's resource type: `"forbid"` or `"permit"`.    |
| `actions`             | array of {ref}`DiagnosticsActionPolicies <permission-service-diagnostics-diagnosticsactionpolicies>` | yes | One entry per action in the batch, in request order. |

(permission-service-diagnostics-diagnosticsactionpolicies)=
### DiagnosticsActionPolicies

Candidate policies for one action of a batch entry.

| Field      | Type                              | Required | Description                                               |
|------------|-----------------------------------|----------|-----------------------------------------------------------|
| `service`  | string                            | yes      | Service of the action, echoing the request.               |
| `action`   | string                            | yes      | Action name, echoing the request.                         |
| `policies` | array of {ref}`DiagnosticsPolicy <permission-service-diagnostics-diagnosticspolicy>` | yes | Candidate policies for this action, in evaluation order. |

(permission-service-diagnostics-post-v1beta-diagnostics-authorize)=
## POST /v1beta/diagnostics/authorize/ — inspect a single request

Reports the candidate policies and the evaluation priority for one principal, action, and resource combination. The request body is the same `AuthorizationPayload` the authorization API accepts, so a payload that produced an unexpected decision can be replayed here unchanged. The response is a {ref}`DiagnosticsAuthorizeResponse <permission-service-diagnostics-diagnosticsauthorizeresponse>`.

(permission-service-diagnostics-post-v1beta-diagnostics-authorize-example)=
### Example

```http
POST /v1beta/diagnostics/authorize/ HTTP/1.1
Host: permission-service.storage-apis.svc.cluster.local:3000
Authorization: Bearer eyJraWQiOiJvYXV0aC1zaWduL[...]cc5IgUXhY66ML-CZVlRw
Content-Type: application/json

{
   "principal": {"sub": "DdxA9xDiqdUbv", "info": {"email": "user@test.com"}},
   "action": {
      "name": "read",
      "service": "storage"
   },
   "resource": {
      "id": "/Projects/Scene.usd",
      "type": "File",
      "data": {
         "metadata": {
            "size": 1024
         }
      }
   }
}
```

```http
HTTP/1.1 200 OK
Content-Type: application/json

{
   "evaluation_priority": "forbid",
   "policies": [
      {
         "id": 1,
         "order": 0,
         "policy": "permit(principal, action == Action::\"storage:read\", resource);",
         "action": {"name": "read", "service": "storage"}
      },
      {
         "id": 5,
         "order": 10,
         "policy": "forbid(principal == Principal::\"DdxA9xDiqdUbv\", action, resource);",
         "principal": {"sub": "DdxA9xDiqdUbv"},
         "resource": {"id": "/Projects/Scene.usd", "type": "File", "data": {}}
      }
   ]
}
```

(permission-service-diagnostics-post-v1beta-diagnostics-authorize-batch)=
## POST /v1beta/diagnostics/authorize/batch/ — inspect a batched request

Reports the candidate policies and evaluation priority for every action of every batch entry. The request body is the same `AuthorizationBatchRequest` the batch authorization API accepts, including its optional `condition` field; diagnostics never short-circuits, so `condition` has no effect on the reported policies. The response is a {ref}`DiagnosticsAuthorizeBatchResponse <permission-service-diagnostics-diagnosticsauthorizebatchresponse>`.

(permission-service-diagnostics-post-v1beta-diagnostics-authorize-batch-example)=
### Example

```http
POST /v1beta/diagnostics/authorize/batch/ HTTP/1.1
Host: permission-service.storage-apis.svc.cluster.local:3000
Authorization: Bearer eyJraWQiOiJvYXV0aC1zaWduL[...]cc5IgUXhY66ML-CZVlRw
Content-Type: application/json

{
   "batches": [
      {
         "principal": {"sub": "DdxA9xDiqdUbv"},
         "actions": [
            {"name": "read", "service": "storage"},
            {"name": "write", "service": "storage"}
         ],
         "resource": {"id": "/Projects/Scene.usd", "type": "File"}
      }
   ]
}
```

```http
HTTP/1.1 200 OK
Content-Type: application/json

{
   "results": [
      {
         "resource": {"id": "/Projects/Scene.usd", "type": "File", "data": {}},
         "evaluation_priority": "forbid",
         "actions": [
            {
               "service": "storage",
               "action": "read",
               "policies": [
                  {
                     "id": 1,
                     "order": 0,
                     "policy": "permit(principal, action == Action::\"storage:read\", resource);",
                     "action": {"name": "read", "service": "storage"}
                  }
               ]
            },
            {
               "service": "storage",
               "action": "write",
               "policies": []
            }
         ]
      }
   ]
}
```

(permission-service-diagnostics-how-the-service-serves-these-endpoints)=
## How the service serves these endpoints

For each request — and, in the batched variant, for each action of each batch entry — the service:

1. Canonicalizes the submitted resource, applying the {ref}`URL remapper <permission-service-deployment-url-remapper>` when one is configured, and URL-encodes the resource id the same way a real authorization request does.
2. Verifies the caller holds `permissions:diagnostics`, and rejects the request before reading any policy otherwise.
3. Resolves the resource type's **evaluation priority** from the metadata store, falling back to `forbid` when the resource type is not registered. See {ref}`Evaluation priority for resources <permission-service-metadata-evaluation-priority-for-resources>`.
4. Fetches the **candidate policies** whose scopes match the principal, group memberships, action, and resource, using the same retrieval filter as a real authorization request.
5. Returns the priority and the candidate policies, sorted by `order` ascending and then by `id` ascending — the ordering the evaluation pipeline applies.

Because the lookups are the same ones a live request performs, a diagnostics call reflects the cached metadata and stored policies the next authorization request would use. It does not run Cedar, so it reports no decision, and it does not populate the evaluation cache.

For how the reported priority and ordering combine into an allow or deny decision, see {ref}`How authorization requests are evaluated <permission-service-policy-evaluation-how-authorization-requests-are-evaluated>`.

(permission-service-diagnostics-see-next)=
## See next

- {ref}`Service notifications <permission-service-notifications-service-notifications>` — events published when the policies reported here change.

Related pages:

- {ref}`How authorization requests are evaluated <permission-service-policy-evaluation-how-authorization-requests-are-evaluated>` — the pipeline and Cedar semantics behind every authorization call.
- {ref}`Managing policies through the REST API <permission-service-policy-management-managing-policies-through-the-rest-api>` — how the reported policies are created and maintained.
- {ref}`Permission Web UI <permission-service-web-ui-permission-web-ui>` — the Diagnostics page that presents this API as a form.
