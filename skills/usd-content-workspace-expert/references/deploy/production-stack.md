# Production stack (S3 / Azure Blob Storage)

> **Applies to:** NVIDIA Storage Service (S3/Azure production adapter), Auto Discovery, Contour ingress, Storage Navigator, plus how notifications, auth, permissions, user info, thumbnails, and Live Edit plug in · **Released:** Developer Guide 1.3.0 inventory — `storage-service` 1.2.3, `auto-discovery-service` 1.1.1, `storage-navigator` 1.0.4 (others in [released-inventory.md](../versions/released-inventory.md)) · **This repo's snapshot:** Storage Service 1.2.3 + `[Unreleased]` ([`CHANGELOG.md`](../../../../storage/reference/storage-service/CHANGELOG.md)); operator 1.2.0 ([`Cargo.toml`](../../../../discovery/reference/discovery-service/operator/Cargo.toml)); Navigator 1.0.4 ([`package.json`](../../../../examples/navigator-ui/package.json)) · **Sources:** the [source skill package](../../NOTICE.md); [`storage/reference/storage-service/docs/`](../../../../storage/reference/storage-service/docs/deployment.rst); chart `values.yaml` and templates linked below

The production stack uses the NVIDIA Storage Service in front of S3, S3-compatible stores, or Azure Blob Storage, and composes other services around it. It runs on any conformant Kubernetes distribution (EKS, AKS, GKE, on-premises). Start with Storage and Discovery; add the rest as the workflow needs them, and replace any reference service with your own implementation of its contract ([../stack/replaceable-and-modular.md](../stack/replaceable-and-modular.md)). For a single workstation, a local cluster ([local-development.md](local-development.md)) or plain local processes ([local-process.md](local-process.md)) are lighter options.

Before you configure anything, establish which versions you are deploying ([../versions/capture-a-deployment.md](../versions/capture-a-deployment.md) for an existing deployment, [released-inventory.md](../versions/released-inventory.md) for a new one), then read that chart's `helm show values` output and the component `CHANGELOG.md`/migration guide. Keys in this repository's snapshot charts can differ from the released charts.

Install production from the prebuilt containers and Helm charts in the [usd-content-workspace NGC collection](https://catalog.ngc.nvidia.com/orgs/nvidia/teams/omniverse/collections/storage_apis) unless you own builds of your own ([artifact-access.md](artifact-access.md) for access and pull secrets, [building-from-source.md](building-from-source.md) otherwise).

| Read | For |
| --- | --- |
| [production-stack/storage-service.md](production-stack/storage-service.md) | Backend choice, IAM/RBAC, S3 / S3-compatible / Azure values, workload identity, install, Discovery registration, validation |
| [production-stack/ingress-and-tls.md](production-stack/ingress-and-tls.md) | Contour, DNS, certificates, per-chart `httpProxy` values, the shared ingress overlay, enabling the auth extension |
| [production-stack/validation.md](production-stack/validation.md) | End-to-end checks from inside the cluster, through ingress, with authentication, and from Kit |
| [discovery.md](discovery.md) | Auto Discovery operator (and the legacy static chart) |
| [navigator.md](navigator.md) | Storage Navigator |
| [script-templates.md](script-templates.md) | Repeatable per-service scripts and values layout |

## Layers

| Layer | Services | Required | Reference |
| --- | --- | --- | --- |
| Core | Storage Service, Discovery | Yes | [production-stack/storage-service.md](production-stack/storage-service.md), [discovery.md](discovery.md) |
| Ingress | Contour (HTTPProxy) | For clients outside the cluster | [production-stack/ingress-and-tls.md](production-stack/ingress-and-tls.md) |
| Authentication | Envoy auth extension (OIDC) | For any exposed deployment | [permissions-and-auth.md](permissions-and-auth.md) |
| Authorization | Permission Service, Permission UI | When access policy is enforced | [permissions-and-auth.md](permissions-and-auth.md) |
| Identity lookup | Microsoft Graph User Info Service | For user/group display and lookup | [userinfo.md](userinfo.md) |
| Notifications | RabbitMQ, Event Aggregation, Event Consumer | For event-driven clients | [notifications.md](notifications.md), [../operate/notifications.md](../operate/notifications.md) |
| Thumbnails | Thumbnail Service and task worker | For previews | [thumbnails.md](thumbnails.md) |
| Live collaboration | Live Edit service (released chart 0.1.1; source not in this repository) | For live sessions | [live-edit.md](live-edit.md) |
| UI | Storage Navigator | Optional | [navigator.md](navigator.md) |
| Tests | Storage API integration tests (released chart) | Recommended after changes | [../test/integration-tests.md](../test/integration-tests.md) |

## Order of installation

Each step depends on the ones before it. Validate each before moving on.

1. **Namespace and pull secret** — [artifact-access.md](artifact-access.md#image-pull-secrets).
2. **Backend prerequisites** — bucket/container, IAM role or Azure role, CORS if browsers will reach the backend directly ([production-stack/storage-service.md](production-stack/storage-service.md)).
3. **Storage Service** — values, install, in-cluster validation.
4. **Discovery** — Auto Discovery operator; annotate Storage's Service ([discovery.md](discovery.md)). Check the internal catalog.
5. **Ingress and TLS** — Contour, DNS, certificates; enable `httpProxy` on Storage and Discovery; set Discovery external endpoints. Check the external catalog from a client network.
6. **Authentication** — OIDC app registration, Envoy auth extension, Discovery `openId` (auth bootstrap), `httpProxy.authExtension.enabled` on each protected chart ([permissions-and-auth.md](permissions-and-auth.md)). Confirm unauthenticated requests get 401 and `auth-config` stays public.
7. **Authorization** — Permission Service, then `config.permissions.enabled` on Storage ([permissions-and-auth.md](permissions-and-auth.md)). Seed administrator meta-permissions before relying on policy administration.
8. **Notifications** — RabbitMQ, Event Aggregation, Event Consumer; backend event source (SQS / Service Bus) and `config.notificationClient` on Storage ([notifications.md](notifications.md)).
9. **User Info, Thumbnails, Live Edit** — as needed ([userinfo.md](userinfo.md), [thumbnails.md](thumbnails.md), [live-edit.md](live-edit.md)). Live Edit needs an authenticated Storage Service holding the session base assets.
10. **Navigator** — after Discovery, ingress, and auth exist ([navigator.md](navigator.md)).
11. **Integration tests** — [../test/integration-tests.md](../test/integration-tests.md).

## Prerequisites

From the released guide, as recorded by the source skill package: Kubernetes 1.31+, kubectl 1.32+, Helm 3. The Storage Service PostgreSQL metadata backend independently needs Kubernetes 1.29+ (native sidecar for the token provisioner; [`storage-metadata.rst`](../../../../storage/reference/storage-service/docs/helm/storage-metadata.rst)). Compute nodes suffice; no GPUs are needed for this layer. The guide's example sizing is 32 vCPU / 128 GiB nodes (AWS m5.8xlarge, Azure Standard_D32as_v4); size from measured load ([../operate/scalability.md](../operate/scalability.md)).

## Layered values

Each Helm install takes several `-f` files; later files override earlier ones:

1. Per-service base values (`storage-values.yaml`, `discovery-values.yaml`, `navigator-values.yaml`, ...).
2. A shared `ingress-values.yaml` with `httpProxy.enabled`, `httpProxy.fqdn.domain`, `httpProxy.tls.*`, and `httpProxy.authExtension.enabled`. These keys exist with the same meaning in the Storage, Auto Discovery, and Navigator snapshot charts (Navigator has no `authExtension`; extra keys are ignored). Check every other chart's schema before applying the shared overlay to it.
3. `oidc-config.yaml` with `openId.*`, applied to Discovery (and to the charts in [permissions-and-auth.md](permissions-and-auth.md) that use the same keys).

Keep every values file in version control without secrets; secrets come from Kubernetes Secrets referenced by the values. See [script-templates.md](script-templates.md).

## Ports

Snapshot chart defaults; confirm against the chart you install.

| Service | gRPC | REST / HTTP | Other | Source |
| --- | --- | --- | --- | --- |
| Storage Service | 8011 (`service.grpc.port`) | 8012 (`service.rest.port`) | metrics 8013 when `service.metrics.enabled`; gRPC probe 8100 | [`values.yaml`](../../../../storage/reference/storage-service/helm/values.yaml), [`deployment.yaml`](../../../../storage/reference/storage-service/helm/templates/storage-service/deployment.yaml) |
| Auto Discovery | — | internal 8080, external 8081 | probes `GET /api/v2/services` on internal | [`values.yaml`](../../../../discovery/reference/discovery-service/operator/helm/auto-discovery-service/values.yaml) |
| Legacy Discovery | — | 8080 | `/health` | [`configmap.yaml`](../../../../discovery/reference/discovery-service/chart/helm/templates/configmap.yaml) |
| Storage Navigator | — | 8080 (`service.port`) | — | [`values.yaml`](../../../../examples/navigator-ui/helm/storage-navigator/values.yaml) |
| Notifications, Permission Service, Permission UI, User Info, Thumbnails, Live Edit | see their deploy references | | | [notifications.md](notifications.md), [permissions-and-auth.md](permissions-and-auth.md), [userinfo.md](userinfo.md), [thumbnails.md](thumbnails.md), [live-edit.md](live-edit.md) |

The source skill package lists Navigator on port 80 and Discovery on 8080 only; the snapshot charts use 8080 for Navigator and 8080/8081 (internal/external) for Auto Discovery.

## Secrets this layer uses

| Secret | Keys | Used by |
| --- | --- | --- |
| `ngcpull-secret` (any name) | registry credentials | every chart's pull-secret value |
| Backend credentials, only when static keys are chosen | e.g. `STORAGE_SERVICE_AWS_ACCESS_KEY_ID_1`, `STORAGE_SERVICE_AWS_SECRET_ACCESS_KEY_1`, or `STORAGE_SERVICE_AZURE_STORAGE_KEY_1` | Storage Service via `extraEnvFrom`/`extraEnvs` |
| TLS certificate | `tls.crt`, `tls.key` | Contour HTTPProxy `httpProxy.tls.secretName` |

Prefer workload identity (EKS IRSA, AKS Workload Identity) over static backend keys. Notification, OIDC, Permission Service, and Live Edit secrets are described by their owners' references. Keep NGC, backend, service-identity, and caller credentials separate; none substitutes for another.

## Security baseline

- Do not expose any Storage, Discovery, or Navigator route outside the cluster without TLS and the auth extension; the Storage Service trusts what reaches it.
- Discovery visibility (`internal`/`external`/`both`) controls catalog output only. It is not access control.
- The snapshot Auto Discovery chart creates a cluster-wide ClusterRole even when it watches one namespace; see [discovery.md](discovery.md#rbac-and-least-privilege) for a namespaced alternative.
- Default CORS: Storage renders `allowOrigin: []` as a regex allowing any subdomain of `httpProxy.fqdn.domain`; Auto Discovery allows `*` for `GET`. Set explicit origins when other subdomains or origins are not trusted ([production-stack/ingress-and-tls.md](production-stack/ingress-and-tls.md#cors-defaults)).
- See [../stack/security.md](../stack/security.md) for the stack-wide review.
