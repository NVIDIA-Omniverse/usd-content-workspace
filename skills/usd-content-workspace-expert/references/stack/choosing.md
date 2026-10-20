# Choose components and a deployment path

> **Applies to:** the whole stack · **Released:** Developer Guide 1.3.0 inventory · **This repo's snapshot:** see [../versions/source-snapshot.md](../versions/source-snapshot.md) · **Sources:** the [source skill package](../../NOTICE.md); [`docs/getting_started/choosing_components.md`](../../../../docs/getting_started/choosing_components.md)

Two points frame every choice ([replaceable-and-modular.md](replaceable-and-modular.md)):

- **Every service here is a reference implementation.** Keep it, extend it, or replace it with your own implementation of the same contract to fit your infrastructure.
- **Deployments are modular.** Start with Discovery and Storage, then add services only when a workflow needs them.

## Start from the outcome

| The caller wants to | Minimum set | Add when needed | Read |
| --- | --- | --- | --- |
| Learn the APIs or develop an adapter | Python filesystem example + Discovery on a local cluster | Notifications to test events | [../deploy/local-development.md](../deploy/local-development.md), [../develop/custom-storage-adapter.md](../develop/custom-storage-adapter.md) |
| Run on one workstation without Kubernetes | Discovery + Storage (+ Metadata) as local processes | — | [../deploy/local-process.md](../deploy/local-process.md) |
| Serve S3 or Azure content to Kit users | Storage Service + Discovery + Contour with TLS | Envoy auth extension, then Permission Service | [../deploy/production-stack.md](../deploy/production-stack.md) |
| Authenticate users | Envoy auth extension with the organization's OIDC provider | — | [../deploy/permissions-and-auth.md](../deploy/permissions-and-auth.md) |
| Control who can read or write which paths | Permission Service + Storage `config.permissions.enabled: true` | User Info Service for names and groups; Permission UI for administrators | [../deploy/permissions-and-auth.md](../deploy/permissions-and-auth.md), [../deploy/userinfo.md](../deploy/userinfo.md) |
| React to file changes (pipelines, UIs, caches) | Event Aggregation + RabbitMQ + Event Consumer | Durable queues | [../deploy/notifications.md](../deploy/notifications.md) |
| Show previews in browsers or apps | Thumbnail Service and workers | — | [../deploy/thumbnails.md](../deploy/thumbnails.md) |
| Collaborate live on USD layers | Live Edit Service (released 0.1.1) + Valkey + Discovery | ovlive client runtime | [../deploy/live-edit.md](../deploy/live-edit.md) |
| Browse and manage files in a browser | Storage Navigator | Permission UI | [../deploy/navigator.md](../deploy/navigator.md) |
| Move from Nucleus | Migration tool (released 0.1.0) + target Storage deployment | — | [../operate/migration.md](../operate/migration.md) |
| Put their own service behind Discovery and auth | Discovery + Envoy auth extension | Permission Service, notifications | [../develop/add-your-service.md](../develop/add-your-service.md) |
| Replace an NVIDIA service with their own | The API contract and its conformance suite | — | [../develop/](../develop/), [../test/conformance.md](../test/conformance.md) |

Discovery is in every deployment: clients find all other services through it.

## Released artifacts or this repo's source

Prebuilt containers and Helm charts for released versions are in the [usd-content-workspace NGC collection](https://catalog.ngc.nvidia.com/orgs/nvidia/teams/omniverse/collections/storage_apis). Recommend them by default; access is in [../deploy/artifact-access.md](../deploy/artifact-access.md).

| Use released NGC artifacts when | Build from this repo when |
| --- | --- |
| Installing a supported deployment | Changing or extending a reference service |
| The released inventory has the needed features | You need an unreleased fix in the snapshot, and accept that it is unreleased |
| You want NVIDIA-built, scanned and signed images | Running conformance tests against your own implementation |

Building from source produces images you own and must patch. Some components need private registries or toolchains described in [../deploy/building-from-source.md](../deploy/building-from-source.md). Do not mix a source-built image with a chart from a different version without reading both changelogs.

## Where it runs

| Target | Use for | Read |
| --- | --- | --- |
| Local processes, no Kubernetes (optional) | One developer or workstation; fastest loop; contract experiments | [../deploy/local-process.md](../deploy/local-process.md) |
| Local cluster (MicroK8s, kind) | Developing charts, adapters, and multi-service behavior as deployed | [../deploy/local-development.md](../deploy/local-development.md) |
| Production cluster (EKS, AKS, GKE, on-premises) | Shared, secured, operated deployments | [../deploy/production-stack.md](../deploy/production-stack.md) |

Clients connect the same way to all three, through a Discovery URL, so moving between them does not change client code.

## Order of installation (production)

1. Cluster prerequisites: Contour (Envoy ingress), cert-manager or your TLS process, storage classes, and a metadata database (PostgreSQL, or CosmosDB where the selected Storage Service version supports it).
2. Auto Discovery, so later services register as they start.
3. Storage Service with backend credentials from workload identity or Kubernetes Secrets.
4. Envoy auth extension, then switch ingress routes to require it.
5. Permission Service with initial policies; then enable permissions in Storage. Add User Info and Permission UI.
6. Notifications: RabbitMQ, Event Aggregation, Event Consumer; then enable Storage event publishing.
7. Optional: Thumbnails, Live Edit, Navigator.
8. Validate: direct HTTP checks, `storage-api-integration-tests`, and a client check through Kit with `OMNI_STORAGE_DISCOVERY` or Navigator ([../test/integration-tests.md](../test/integration-tests.md)).

A pod reaching `Running` does not prove the API works; validate the operation the caller needs.

## Companion products

Caches (OVCC, OVDC, Hub Workstation Cache) and WRAPP packaging work with this stack but are separate products. See [../use/companion-products.md](../use/companion-products.md).
