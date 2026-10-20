(userinfo-service-index-omniverse-user-info-service)=
# Omniverse User Info Service

(userinfo-service-index-overview)=
## Overview

This service is the **reference implementation** of the Omniverse User Info API — the runnable service that implements the published REST and gRPC contracts so other teams can validate integrations and ship compatible alternatives.

User Info Service is a centralized, read-only identity service for the NVIDIA Omniverse cloud platform. It gives every other service a single place to answer questions such as "who is this user?", "what groups do they belong to?", and "which user or group matches this name?" — without each service integrating with the organization's identity provider on its own.

The service does not own the directory. It serves a continuously synchronized replica of the organization's identity provider (currently Microsoft Entra ID) and exposes user and group lookups, name and email searches, and group-membership queries through two equivalent APIs. Callers look up users by identifier or email, search users and groups by name (prefix and substring), resolve group memberships in both directions, list the members of a group, and identify the currently authenticated caller from their access token.

(userinfo-service-index-table-of-contents)=
## Table of Contents

- {ref}`User Info Service Configuration <userinfo-service-deployment-configuration>`
- {ref}`Database Configuration <userinfo-service-database-database-configuration>`
- {ref}`Directory Synchronization and Data Freshness <userinfo-service-directory-synchronization-directory-synchronization-and-data-freshness>`
- {ref}`Microsoft Entra ID Integration <userinfo-service-identity-provider-microsoft-entra-id-integration>`
- {ref}`Authorization with the Permission Service <userinfo-service-authorization-authorization-with-the-permission-service>`
- {ref}`Directory Scoping <userinfo-service-directory-scoping-directory-scoping>`
- {ref}`Static Directory Overlay <userinfo-service-static-directory-static-directory-overlay>`
- {ref}`Scaling and High Availability <userinfo-service-scaling-scaling-and-high-availability>`
- {ref}`Observability <userinfo-service-observability-observability>`
- {ref}`API Access Logs <userinfo-service-access-logs-api-access-logs>`
- {ref}`Troubleshooting and Error Behavior <userinfo-service-troubleshooting-troubleshooting-and-error-behavior>`

(userinfo-service-index-use-cases)=
## Use Cases

- **Centralized identity retrieval** — Omniverse services obtain user and group information from one service instead of integrating with the organization's identity provider directly. This hides provider-specific authentication, query syntax, pagination, rate limiting, and data-model differences behind a uniform interface, and it is configured once per platform deployment.
- **Stable identifiers for authorization** — the service resolves mutable references such as email addresses and group display names to the immutable identifiers assigned by the identity provider, so that authorization policies and migrated metadata remain valid across name changes, domain migrations, and provider switches.
- **Human-readable display and autocomplete** — the service resolves opaque identifiers back into display names, email addresses, and job titles for presentation in product interfaces, and supports real-time name search (prefix and substring) so administrators can find and select users and groups without knowing their identifiers.
- **Optional authorization enforcement** — when integrated with the Permission Service, each request is authorized against a named action before it runs, and list results are filtered per item so callers only see the users and groups they are permitted to see.

(userinfo-service-index-architecture)=
## Architecture

The diagram below shows how User Info Service keeps a local replica of the organization directory synchronized from **Microsoft Entra ID** through the **Microsoft Graph API**, persists it to **PostgreSQL**, and serves it to Omniverse services over **REST** and **gRPC**.

```{image} images/service_architecture.png
:alt: Reference architecture with the writer synchronizing from Microsoft Entra ID and readers serving Omniverse services
```

The service runs in one of two roles, deployed as separate workloads that share a common database:

- **Writer** — keeps the local directory replica aligned with the identity provider. It performs an initial full synchronization and then periodically fetches only what has changed, writing the result to the shared database. The writer does not serve directory requests, and only one writer runs at a time because the synchronization is a single sequential stream that cannot be parallelized.
- **Reader** — serves the REST and gRPC APIs. Readers load the replica the writer has persisted, refresh it on a fixed interval, and never contact the identity provider for synchronization. Read capacity scales out simply by adding more reader replicas.

Separating the two roles keeps the write path off the request path: readers become ready and start serving in seconds by loading the persisted replica, rather than waiting for a full directory synchronization to complete. Because each reader holds its own copy of the directory and refreshes independently, callers may briefly observe minor differences between replicas — for example, a newly added user visible on one reader before another. This is expected for the service's eventually-consistent, read-only model.

For deployments that do not use a database, a reader can run in a live mode that sends every request straight to the identity provider. This removes the database dependency but ties availability and latency to the provider, and is intended for initial testing.

(userinfo-service-index-request-flow)=
### Request Flow

1. A client sends a directory request over **REST** or **gRPC**, carrying the caller's **Bearer token**.
2. Token validation is performed by the platform's authentication gateway. When Permission Service integration is enabled, the request is additionally authorized against a named action before it runs; requests the caller is not permitted to make are rejected.
3. The service answers the request from its in-memory directory replica — resolving identifiers, applying name and email filters, and paginating results locally. When caching is disabled for the requested kind of data, the request is served live from the identity provider instead.
4. For list requests with authorization enabled, each returned item is checked individually and any the caller may not see are removed before the response is sent.
5. The user, group, membership, or page of results is returned to the caller.

(userinfo-service-index-how-other-services-interact-with-user-info-service)=
### How Other Services Interact with User Info Service

Calling services integrate through either API:

- **REST / JSON** — user, group, and membership endpoints with JSON request and response bodies, plus health and readiness probes. The **Omniverse User Info API** documentation describes the contract; it is published separately from the container image.
- **gRPC** — the equivalent set of remote procedure calls defined in the published API contract, for clients that prefer a binary, strongly-typed interface.

Both APIs accept the same inputs and return the same information, so behavior is identical regardless of which one a caller uses. The current-user request takes the caller's identity from their access token and returns the matching directory record.

(userinfo-service-index-what-user-info-service-interacts-with)=
### What User Info Service Interacts With

| External System | Role |
|---|---|
| **Microsoft Entra ID** | The organization's identity provider and the source of all directory data. The service reads users, groups, and group memberships from it. |
| **PostgreSQL** | The shared store that carries directory data from the writer to the readers. The writer persists the synchronized replica; readers load it on startup and refresh it periodically. Not required in the live testing mode. |
| **Permission Service** | Optional. When authorization is enabled, the service checks each request — and each item in a list response — against the Permission Service, and denies requests when it cannot obtain a decision. |
| **OpenTelemetry** | Optional. Metrics and traces can be exported to your observability stack using standard OpenTelemetry exporters. |

(userinfo-service-index-identity-provider-integration)=
### Identity Provider Integration

The service reads directory data from Microsoft Entra ID as an application, using its own credentials rather than acting on behalf of a signed-in user. Deployments provide the tenant and application details and either a client secret or a federated workload identity, and grant the application read access to users, groups, and group membership.

The writer keeps the replica current using the identity provider's incremental-change mechanism: the first synchronization reads the whole directory, and later cycles fetch only what has changed since the previous cycle. Requests to the provider respect its rate limits, honoring the retry timing it returns and backing off on transient errors, so the service coexists with other consumers of the same tenant.

By default the service replicates the whole tenant directory. It can instead be scoped to just the users and groups assigned to a specific enterprise application, so that the data it serves matches what downstream consumers expect to see. Operators can also overlay additional users, groups, and memberships that do not come from the identity provider — for example, service accounts — which are served alongside the synchronized data.

For the settings that configure the identity-provider integration, the required directory permissions, and the deployment options above, see {ref}`User Info Service Configuration <userinfo-service-deployment-configuration>`.

(userinfo-service-index-technologies)=
## Technologies

| Area | Overview |
|---|---|
| Deployment | Container image, deployed to Kubernetes with a Helm chart |
| Identity provider | Microsoft Entra ID, read through the Microsoft Graph API |
| APIs | HTTPS REST with JSON and gRPC |
| Data freshness | Continuously synchronized local replica, served with eventual consistency |
| Durable storage | PostgreSQL, shared between the writer and readers |
| Authorization | Optional integration with the Omniverse Storage Permission Service |
| Observability | OpenTelemetry for metrics and traces |

```{toctree}
:maxdepth: 1
:caption: Contents:
deployment
database
directory-synchronization
identity-provider
authorization
directory-scoping
static-directory
scaling
observability
access-logs
troubleshooting
```

(userinfo-service-index-see-next)=
## See next

- {ref}`User Info Service Configuration <userinfo-service-deployment-configuration>`

(userinfo-service-index-see-also)=
## See also

The **Omniverse User Info API** specification (REST and gRPC) is documented separately; this service tracks that contract as its reference implementation.
