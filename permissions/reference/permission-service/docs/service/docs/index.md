(permission-service-index-omniverse-storage-permission-service)=
# Omniverse Storage Permission Service

(permission-service-index-overview)=
## Overview

This service is the **reference implementation** of the Omniverse Storage Permission API — the runnable service that implements the published REST and gRPC contracts so other teams can validate integrations and ship compatible alternatives.

Permission Service is a centralized authorization microservice for the NVIDIA Omniverse Storage API ecosystem.
It evaluates access-control decisions on behalf of other services: given a **principal** (user or service identity),
an **action** (service-scoped operation), and an optional **resource**, the service returns an `allow`, `deny`, or
`skip` decision.

Authorization rules are expressed as [Cedar](https://www.cedarpolicy.com/) policies — a declarative, auditable
policy language. The service stores these policies together with service metadata (registered services, their actions,
resource types, and evaluation priorities), and uses that metadata to resolve caller identity and to drive evaluation
behavior.

(permission-service-index-table-of-contents)=
## Table of Contents

- {ref}`Permission Service Configuration <permission-service-deployment-permission-service-configuration>`
- {ref}`Database Configuration <permission-service-database-configuration-database-configuration>`
- {ref}`Authenticating with an OpenID Connect Identity Provider <permission-service-authentication-authenticating-with-an-openid-connect-identity-provider>`
- {ref}`Writing Cedar policies for Permission Service <permission-service-policy-language-writing-cedar-policies-for-permission-service>`
- {ref}`Managing policies through the REST API <permission-service-policy-management-managing-policies-through-the-rest-api>`
- {ref}`Managing service metadata through the REST API <permission-service-metadata-managing-service-metadata-through-the-rest-api>`
- {ref}`How authorization requests are evaluated <permission-service-policy-evaluation-how-authorization-requests-are-evaluated>`
- {ref}`Diagnostics API <permission-service-diagnostics-diagnostics-api>`
- {ref}`Service notifications <permission-service-notifications-service-notifications>`
- {ref}`Observability <permission-service-observability-observability>`
- {ref}`API access logs <permission-service-access-logs-api-access-logs>`
- {ref}`Permission Web UI <permission-service-web-ui-permission-web-ui>`

(permission-service-index-use-cases)=
## Use Cases

- **Unified access control** — Omniverse services delegate authorization to a single service instead of
  implementing their own policy logic. This keeps authorization rules consistent and manageable across the platform.
- **Fine-grained policies** — Cedar policies can match on principal, action, resource, and arbitrary context
  attributes, enabling rules that range from broad ("allow all reads") to precise ("allow user X to write
  document Y").
- **Batch authorization** — Callers can check multiple action/resource combinations in a single request
  with configurable short-circuit behavior, reducing round-trips for complex permission checks.
- **Meta-permissions** — When authentication is enabled, the service enforces policies that control who can
  view or modify policies and service metadata themselves, providing self-protecting administration.

(permission-service-index-architecture)=
## Architecture

The diagram below uses **Storage Service** as a concrete example of an Omniverse Storage client that calls Permission Service over REST or gRPC. Other services in the ecosystem follow the same pattern.

```{image} images/architecture-storage-service-example.png
:alt: Reference architecture with Storage Service calling Permission Service
```

(permission-service-index-request-flow)=
### Request Flow

1. A client such as **Storage Service** sends an authorization request over **REST** or **gRPC** with a
   principal, action, and optional resource.
2. If authentication is enabled, the service validates the caller's **Bearer token** against the configured
   OpenID Connect provider using **JWKS** and/or the **userinfo** endpoint, depending on how token validation is
   configured (see {ref}`OpenID Connect Identity Provider <permission-service-index-openid-connect-identity-provider>` below).
3. The service resolves **service metadata** (held in memory) to determine the principal id claim for the target
   service and the evaluation priority for the matching resource type. Services and resource types that are not
   registered fall back to default metadata rather than being rejected.
4. Relevant **Cedar policies** are loaded from storage and evaluated by the embedded Cedar engine. Evaluation
   results may be cached to accelerate repeated checks.
5. The decision (`allow`, `deny`, or `skip`) is returned to the caller.

For a detailed walk-through of each stage — authentication, metadata and cache lookups, policy retrieval, and Cedar's order-grouped evaluation — see {ref}`How authorization requests are evaluated <permission-service-policy-evaluation-how-authorization-requests-are-evaluated>`.

(permission-service-index-how-other-services-interact-with-permission-service)=
### How Other Services Interact with Permission Service

Calling services integrate through either API:

- **REST / JSON** — Single and batch authorization endpoints with JSON request and response bodies. Interactive
  API documentation is bundled with the service.
- **gRPC** — Single and batch permission-check RPCs defined in the published API contract; reflection is available
  for clients that discover services dynamically.

Both APIs accept the same logical inputs: principal, service-scoped action, optional resource, and optional context.
Both return a decision with an optional reason.

(permission-service-index-what-permission-service-interacts-with)=
### What Permission Service Interacts With

| External System | Role |
|---|---|
| **PostgreSQL** | Read-write storage for policies and service metadata. An alternative **config-file** mode uses local YAML files as a read-only store (no database required). |
| **OpenID Connect Identity Provider** | Validates incoming bearer tokens when authentication is enabled. The service loads the provider’s discovery document and then uses **JWKS** and/or the **userinfo** endpoint as described below. |
| **Event Aggregation Service** | Optional. When notifications are enabled, the service publishes permission-change events over gRPC so that downstream services can react to policy updates. See {ref}`Service notifications <permission-service-notifications-service-notifications>`. |
| **OpenTelemetry** | Optional. Metrics and traces can be exported to your observability stack using standard OpenTelemetry exporters. |

(permission-service-index-openid-connect-identity-provider)=
### OpenID Connect Identity Provider

When authentication is enabled, Permission Service trusts an OpenID Connect (OIDC) provider discovered at
deployment time. Incoming bearer tokens are validated in one of these ways:

- **JWT with JWKS** — For JWT access tokens, the service verifies the signature and claims using the provider’s
  **JWKS** publishing endpoint. Keys are cached and refreshed so the IdP can rotate signing material without service
  restarts.
- **JWT plus userinfo** — The same **JWKS** verification applies, and the service also calls the OIDC **userinfo**
  endpoint with the bearer token to enrich the principal (for example, extra profile claims used in policies or
  logging).
- **Opaque tokens via userinfo** — For opaque access tokens, the service relies on the **userinfo** endpoint: the
  token is accepted when the provider returns a successful response, and claims come from that response.

Validated tokens and principal details may be cached to limit load on the identity provider.

For the Helm values that configure the IdP integration, the three token verification types in detail, and a worked Microsoft Entra ID example, see {ref}`Authenticating with an OpenID Connect Identity Provider <permission-service-authentication-authenticating-with-an-openid-connect-identity-provider>`.

(permission-service-index-technologies)=
## Technologies

| Area | Overview |
|---|---|
| Deployment unit | Single container image, deployed with the Permission Service Helm chart |
| Policy language | [Cedar](https://www.cedarpolicy.com/) |
| APIs | REST with JSON over plain HTTP (port `3000`), gRPC (port `3010`), and interactive OpenAPI-based documentation; TLS is terminated only at the optional HTTPProxy ingress (`httpProxy.enabled`, off by default) |
| Caching | In-memory caches for evaluation results, metadata, and auth-related data |
| Durable storage | PostgreSQL, or file-backed YAML for read-only deployments |
| Observability | OpenTelemetry for metrics and traces |

```{toctree}
:maxdepth: 1
:caption: Contents:
deployment
database-configuration
authentication
policy-language
policy-management
metadata
policy-evaluation
diagnostics
notifications
observability
access-logs
web-ui
```

(permission-service-index-see-next)=
## See next

- {ref}`Permission Service Configuration <permission-service-deployment-permission-service-configuration>` — deploying the service with the Helm chart and choosing a storage backend.

The **Omniverse Storage Permission API** specification (REST and gRPC) is documented separately; this service tracks that contract as its reference implementation.
