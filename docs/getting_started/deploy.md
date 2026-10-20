<!-- SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Deploy a service

The reference services deploy to Kubernetes with Helm charts kept in each implementation's `helm/` directory. Each chart has its own values, prerequisites, and version; read the chart's documentation before installing, and supply the storage, identity provider, databases, and message brokers it requires.

Prebuilt containers and Helm charts for released versions are in the [usd-content-workspace collection on NGC](https://catalog.ngc.nvidia.com/orgs/nvidia/teams/omniverse/collections/storage_apis). The collection still uses an earlier catalog name and URL, which will change.

## Storage Service

The [Storage Service](../../storage/reference/storage-service/README.md) adapts S3 and Azure Blob Storage into the Storage API. Its [Helm chart reference](../../storage/reference/storage-service/docs/helm/index.rst) documents each group of settings: storage backends and credentials, caching, events, metadata, permissions, manifests, Redis, URL remapping and normalization, HTTP proxy, and observability. If you are upgrading an existing installation, follow its [migration guide](../../storage/reference/storage-service/docs/helm/migration.rst) first.

## Discovery

The [Discovery operator](../../discovery/reference/discovery-service/operator/README.md) watches Kubernetes Services, queries their capabilities, and serves the catalog at `GET /api/v2/services`. Install its chart from the operator directory:

```bash
helm install auto-discovery-service ./helm/auto-discovery-service \
  --set config.kubeNamespace=my-namespace \
  --set config.reconcileInterval=15
```

By default the chart watches only the namespace its Pod runs in; set `config.kubeNamespace` to select others. The [legacy discovery chart](../../discovery/reference/discovery-service/chart/README.md) uses a different configuration and response format; use the guide for the deployment you select.

## Other services

| Service | Deployment material |
| --- | --- |
| Permission Service | [README](../../permissions/reference/permission-service/README.md) · [Helm charts](../../permissions/reference/permission-service/helm/) |
| Envoy auth extension | [README](../../permissions/reference/envoy-auth-extension/README.md) · [Helm chart](../../permissions/reference/envoy-auth-extension/helm/) |
| User Info Service | [README](../../user-identity/reference/microsoft-graph-userinfo-service/README.md) · [Helm charts](../../user-identity/reference/microsoft-graph-userinfo-service/helm/) |
| Event Aggregation Service | [README](../../notifications/reference/event-aggregation-service/README.md) · [Helm charts, including RabbitMQ](../../notifications/reference/event-aggregation-service/helm/README.md) |
| Event Consumer Service | [README](../../notifications/reference/event-consumer-service/README.md) · [Helm charts](../../notifications/reference/event-consumer-service/helm/README.md) |
| Thumbnail Service | [README](../../thumbnails/reference/thumbnail-service/README.md) · [Deployment architecture](../../thumbnails/reference/thumbnail-service/helm/DEPLOYMENT_ARCHITECTURE.md) |

Several services ship a conformance-test chart next to the service chart, so you can check a deployment in the cluster it runs in.
