---
name: storage-service
description: Locate the right Omniverse Storage Service documentation fast. Use when configuring, deploying, upgrading or debugging the NVIDIA Storage Service (the S3/Azure production storage adapter) — Helm values, storage backends, credentials, caching, metadata backends, events, permissions, manifests, URL remapping, observability, throttling, scalability or migration. Routes to the authoritative docs rather than restating them.
version: 1.2.2
---

# Storage Service — documentation map

**This skill is a table of contents, not a copy of the documentation.** It exists so an agent
can find the authoritative page in one hop instead of reading the whole doc set. Read the linked
page before answering; do not answer configuration questions from this file alone.

The documentation lives beside the code in this repository under `docs/`, and is published with
each release. Section names below match the published titles.

## Where the product lives

| Thing | Where |
|---|---|
| API spec, proto + OpenAPI, conformance suite | [NGC `storage-api` resource](https://catalog.ngc.nvidia.com/orgs/nvidia/teams/omniverse/resources/storage-api) |
| All Storage APIs components | [NGC `storage_apis` collection](https://catalog.ngc.nvidia.com/orgs/nvidia/teams/omniverse/collections/storage_apis) |
| Helm chart | `https://helm.ngc.nvidia.com/nvidia/omniverse/charts/storage-service-<version>.tgz` |
| Chart values reference | `docs/helm/index.rst` |

## Route by task

| If the question is about… | Go to |
|---|---|
| Deploying the service end to end | `docs/deployment.rst` |
| Which chart value does what | `docs/helm/index.rst` |
| S3 / Azure Blob backend setup, endpoints, addressing | `docs/helm/storage-backend.rst` |
| **Backend** auth — static keys, AKS Workload Identity, EKS IRSA | `docs/helm/storage-backend.rst` |
| **Service identity** — access-token-file sidecar, OAuth2 client secret | `docs/helm/storage-credentials.rst` |
| Response caching, stat/list cache, Redis L2 tier | `docs/helm/storage-backend-caching.rst` |
| Metadata backend — PostgreSQL, CosmosDB (incl. its own workload identity) | `docs/helm/storage-metadata.rst` |
| Storage event subscriptions, synthetic directory events (incl. its own workload identity) | `docs/helm/storage-backend-events.rst` |
| Permission Service integration, per-endpoint enablement | `docs/helm/storage-permissions.rst` |
| Workspace manifests | `docs/helm/manifest.rst` |
| URL remapping and aliases | `docs/helm/url-remapper.rst` |
| Metrics, tracing, structured logging (OTLP) | `docs/helm/observability.rst` |
| Redis / Valkey client configuration | `docs/helm/redis-client.rst` |
| Request throttling and egress limits | `docs/helm/request-throttling.rst` |
| Replicas, autoscaling, resource sizing | `docs/helm/scalability.rst` |
| Ingress / Contour HTTPProxy, route timeouts | `docs/helm/http-proxy.rst` |
| `${ENV_VAR}` interpolation in values | `docs/helm/env-var-interpolation.rst` |
| Value syntax — durations, sizes, booleans | `docs/helm/value-types.rst` |
| Upgrading across versions, removed properties | `docs/helm/migration.rst` |
| Probes, security context, anything uncategorised | `docs/helm/misc.rst` |

## Upgrade traps

Flag these when advising on an upgrade — each one fails quietly rather than erroring.

- **`enablePermissions` was renamed to `permissionsEnabled`** (`enable_permissions` →
  `permissions_enabled`). An older values file keeps the old key and silently loses the setting.
  See `docs/helm/storage-permissions.rst`.
- **Cross-backend `Copy` / `Move` now returns `UNIMPLEMENTED` (REST 501)**, previously
  `INVALID_ARGUMENT`. Callers matching on the old code will not notice until they do.
- **The PostgreSQL metadata backend requires Kubernetes 1.29+** — migrations run from an init
  container and the token provisioner is a native sidecar. See `docs/helm/storage-metadata.rst`.
- **Deprecated chart properties were removed.** `docs/helm/migration.rst` is the checklist; do not
  guess whether a value still exists.

## Scope

This service is the **S3/Azure production storage adapter** — gRPC on 8011, REST on 8012. It is
not the Python example adapter (different ports, not production). If the question is about the
example adapter, a custom adapter, or another component (Discovery, Notifications, Permissions,
Navigator), this is the wrong skill.
