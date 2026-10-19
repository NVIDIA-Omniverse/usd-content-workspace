# Omniverse User Info Service

User Info Service is a centralized, read-only identity service for the NVIDIA Omniverse cloud
platform. It gives every other service a single place to answer questions such as "who is this
user?", "what groups do they belong to?", and "which user or group matches this name?" — without
each service integrating with the organization's identity provider on its own.

The service does not own the directory. It serves a continuously synchronized replica of the
organization's identity provider (currently Microsoft Entra ID, read through the Microsoft Graph
API) and exposes it through two equivalent APIs — **REST/JSON** (`/api/v1alpha/...`) and
**gRPC** (`nvidia.omniverse.userinfo.v1alpha.UserInfoService`). Callers look up users by identifier
or email, search users and groups by name (prefix and substring), resolve group memberships in both
directions, list the members of a group, and identify the currently authenticated caller from their
access token.

## Documentation

Full documentation lives in the [`docs/`](docs/) folder.

### API contract

The published REST and gRPC contract that this service implements:

- [Overview and introduction](docs/api/docs/index.md)
- [Use cases](docs/api/docs/use-cases.md)
- [Authentication and authorization](docs/api/docs/authentication-and-authorization.md)
- [REST API](docs/api/docs/rest-api.md)
- [gRPC specification](docs/api/docs/grpc.md)
- [Directory synchronization and data freshness](docs/api/docs/directory-synchronization.md)
- [Reference documents](docs/api/docs/references.md)

### Service and operations

How the reference implementation works and how to operate it:

- [Overview and architecture](docs/service/docs/index.md)
- [Deployment and configuration](docs/service/docs/deployment.md)
- [Database](docs/service/docs/database.md)
- [Directory synchronization](docs/service/docs/directory-synchronization.md)
- [Identity provider integration](docs/service/docs/identity-provider.md)
- [Authorization](docs/service/docs/authorization.md)
- [Directory scoping](docs/service/docs/directory-scoping.md)
- [Static directory overlay](docs/service/docs/static-directory.md)
- [Scaling and high availability](docs/service/docs/scaling.md)
- [Observability](docs/service/docs/observability.md)
- [Troubleshooting](docs/service/docs/troubleshooting.md)

### Design

- [Software Design Document](docs/sdd.md)

## Repository layout

| Path | Description |
|---|---|
| [`crates/microsoft-graph-userinfo-service`](crates/microsoft-graph-userinfo-service) | The service — the reference implementation of the User Info API. |
| [`crates/userinfo-conformance-test`](crates/userinfo-conformance-test) | Conformance test that validates any implementation against the API contract. |
| [`crates/microsoft-graph-userinfo-smoke-test`](crates/microsoft-graph-userinfo-smoke-test) | Smoke test for a deployed service. |
| [`helm/microsoft-graph-userinfo-service`](helm/microsoft-graph-userinfo-service) | Helm chart for deploying the service to Kubernetes. |
| [`docs/`](docs/) | API, service, and design documentation. |

## License

Licensed under the [Apache License, Version 2.0](http://www.apache.org/licenses/LICENSE-2.0).
