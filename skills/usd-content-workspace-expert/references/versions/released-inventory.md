# Released inventory (NGC)

> **Applies to:** NGC artifacts published with the [usd-content-workspace Developer Guide](https://docs.omniverse.nvidia.com/ovstorage/ovstorage-guide/latest/index.html) 1.3.0 · **Recorded:** NGC inventory observed 2026-10-07 · **Sources:** the [source skill package](../../NOTICE.md) release pins and changelog; [usd-content-workspace NGC collection](https://catalog.ngc.nvidia.com/orgs/nvidia/teams/omniverse/collections/storage_apis)

Use this set for new installations from NGC unless the caller names another. The documentation version (1.3.0) names the release; containers, charts, and API resources each keep their own versions. A `latest` documentation URL can advance past an installed deployment; check the version shown on the page.

## Current release (Developer Guide 1.3.0)

| Artifact (chart / image when different) | Chart | Image | This repo's snapshot |
| --- | --- | --- | --- |
| `storage-service` | 1.2.3 | 1.2.3 | 1.2.3 + unreleased changes |
| `auto-discovery-service` | 1.1.1 | 1.1.1 | 1.2.0, unreleased |
| `discovery-service` (legacy static catalog) | 2.3.11 | — | 2.4.1, unreleased |
| `simple-nginx` (legacy/example Discovery image) | — | 1.1.0 | not in repo |
| `event-aggregation-service` | 1.6.0 | 1.6.0 | 1.7.0 |
| `event-consumer-service` | 1.11.0 | 1.11.0 | 1.12.0 |
| `rabbitmq` | 99.3.0 | 4.1.3-debian-12-r1 | chart under `notifications/reference/event-aggregation-service/helm/rabbitmq` |
| `envoy-auth-extension` / `envoy-auth-ext` | 2.4.0 | 2.4.0 | 2.4.1, unreleased |
| `permission-service` | 0.0.53 | 0.0.53 | 0.0.54, unreleased |
| `permission-ui` | 0.0.22 | 0.0.22 | 0.0.22 / package 0.0.23 |
| `microsoft-graph-userinfo-service` | 0.0.8 | 0.0.8 | 0.0.9 |
| `storage-navigator` | 1.0.4 | 1.0.4 | 1.0.4 |
| `storage-api-integration-tests` / `integration-tests` | 1.0.23 | 1.0.23 | not in repo |
| `smoke-test-base` | — | 1.0.9 | not in repo |
| `live-edit-service` | 0.1.1 | 0.1.1 | not in repo (client runtime is ovlive) |
| `thumbnail-service` | 0.1.0 | 0.1.0 | 0.1.0 |
| `migration-tool` (Nucleus migration) | — | 0.1.0 | not in repo |

### API resources

| NGC API resource | Version | Wire packages in this repo |
| --- | --- | --- |
| `storage-api` | 1.0.0-beta.4 (published HTML docs: 1.0.0-beta.3) | `nvidia.omniverse.storage.*.v1alpha` / `v1beta`; repo snapshot unreleased (after 1.0.0-beta.4) |
| `notifications-aggregation-api` | 1.0.0-beta.3 | `nvidia.omniverse.notifications.publisher.v1beta` |
| `notifications-consumer-api` | 1.0.0-beta.3 | `nvidia.omniverse.notifications.consumer.v1beta` |
| `permission-api` | 1.0.0-beta.1 ([HTML docs](https://docs.omniverse.nvidia.com/ovstorage/permission-api/1.0.0-beta.1/index.html) match) | `nvidia.omniverse.permission.v1beta` |
| `userinfo-api` | 1.0.0-alpha.1 | `nvidia.omniverse.userinfo.v1alpha` |

See [api-contracts.md](api-contracts.md) for how resource, documentation, and wire versions relate.

## Release notes for 1.3.0

The release adds Live Edit 0.1.1, a public Thumbnail Service 0.1.0, and Nucleus Migration Tool 0.1.0. Navigator 1.0.4 adds Discovery v2 with v1 fallback. Notifications add request and pool controls, durable-queue bounds and expiry, shared IDs, negative acknowledgements, shutdown handling, and observability. Integration tests 1.0.23 add pre-provisioned policy and optional-service guidance.

## Known conflicts inside the release

- **Auto Discovery:** NGC supplies 1.1.1; the Developer Guide 1.3.0 changelog still lists 1.1.0. Use the 1.1.1 chart and inspect its values.
- **Legacy versus Auto Discovery:** the legacy chart serves a static catalog; Auto Discovery uses an operator, RBAC, and Service annotations. Renaming a chart or changing its version does not convert one configuration into the other. Some guide examples still describe the static nginx catalog.
- **Catalog route:** the released operator serves `GET /api/v2/services` with `schema-version: 2`. The unreleased snapshot operator serves the same catalog at `/api/v2beta/services` and keeps `/api/v2/services` as an alias. Guide runbooks that use `/v2alpha/services` on Storage describe a different or stale route.
- **RabbitMQ:** the release pins image 4.1.3, while notification operations recommend 4.1.5 or newer for a policy-repair defect. A broker upgrade needs its own validation. Chart `appVersion` (4.0.1) is not the image tag; the chart also defaults to a staging repository prefix, so set `image.registry`, `image.repositoryPrefix`, `image.repository`, and `image.tag` to the entitled image and inspect every rendered image.
- **Smoke tests:** charts in this release pin `smoke-test-base` 1.0.9. Previous-release charts defaulted to 1.0.1 and needed `smokeTest.image.tag: 1.0.7`.
- **Client Library:** the docs release manifest references Client Library 2.72; the source skill package's client examples target 2.74 APIs and authentication behavior and need a Kit or runtime that includes 2.74. Check `omni.client.get_version()` in the actual process. The guide's compatibility table covers Kit 109.x, 110.1, and 110.2 with usd-content-workspace releases 1.0–1.2; it does not certify the 1.3.0 inventory.
- **Migration image:** the inventory names it `migration-tool`; earlier examples used `ov-nucleus-migration`. Verify the registry coordinate before pulling.
- **Thumbnail worker:** the Thumbnail chart references a worker image not separately listed in the inventory; verify it before deployment.

## Previous release (`published` profile)

Keep for existing deployments only. Live Edit, Thumbnail, and Migration are absent from it, and it uses legacy Discovery.

| Artifact | Chart | Image |
| --- | --- | --- |
| `storage-service` | 1.2.0 | 1.2.0 |
| `discovery-service` | 2.3.11 | — |
| `simple-nginx` | — | 1.1.0 |
| `event-aggregation-service` | 1.5.6 | 1.5.6 |
| `event-consumer-service` | 1.10.0 | 1.10.0 |
| `rabbitmq` | 99.3.0 | 4.1.3-debian-12-r1 |
| `envoy-auth-extension` | 2.3.3 | 2.3.3 |
| `permission-service` | 0.0.52 | 0.0.52 |
| `microsoft-graph-userinfo-service` | 0.0.7 | 0.0.7 |
| `storage-navigator` | 1.0.3 | 1.0.3 |
| `permission-ui` | 0.0.22 | 0.0.22 |
| `storage-api-integration-tests` | 1.0.18 | 1.0.18 |
| `smoke-test-base` | — | 1.0.7 |

Previous API resources: `storage-api` 1.0.0-beta.4, `notifications-aggregation-api` and `notifications-consumer-api` 1.0.0-beta.2, `permission-api` 1.0.0-beta.1, `userinfo-api` 1.0.0-alpha.1.

## Historical upgrade boundaries

- **Storage Service 1.0.2 → 1.2.0:** S3, Azure Blob, and Azure Table metadata backends were removed; DynamoDB and CosmosDB were temporarily disabled because batch operations were not implemented for them. PostgreSQL is the supported metadata destination for 1.2.0. Migrations run in an init container before replicas serve traffic, and the token provisioner runs as a native sidecar, which needs Kubernetes 1.29 or later. An image rollback does not reverse a metadata migration. This restriction is specific to 1.2.0: the 1.2.3 reference documents both PostgreSQL and CosmosDB.
- **Storage Service 1.0.2 → 1.2.2:** `enable_permissions` / `enablePermissions` was renamed `permissions_enabled` / `permissionsEnabled`, and values carrying the old name silently lose the setting. `allowUnconfiguredEndpoints: false` rejects requests to unconfigured endpoints. Cross-backend `Copy` and `Move` return `UNIMPLEMENTED` (REST 501). The `observability` block supersedes `service.metrics.enabled`. Discovery v2alpha advertises the REST version mount base (`/v1alpha`).
- **v1.0.0 GA (April 2026)** charts: storage-service 1.0.2, discovery-service 2.3.8, event-aggregation-service 1.5.6, event-consumer-service 1.9.6, envoy-auth-extension 2.3.3, storage-navigator 1.0.1, storage-api-integration-tests 1.0.3, rabbitmq 99.3.0.

For a specific upgrade, follow [../operate/upgrades.md](../operate/upgrades.md).

## Acquiring artifacts

NGC catalog pages can return 200 while chart downloads or image pulls fail. Entitlement, chart download, image pull, and running-service behavior are separate checks; an NGC API key is an acquisition credential, not a runtime identity. See [../deploy/artifact-access.md](../deploy/artifact-access.md).
