# Reference index

Back to the [skill entry point](../SKILL.md). Every page starts with an **Applies to** block naming the components, the released and snapshot versions it covers, and its sources. Confirm the versions you run before applying configuration ([versions/](versions/README.md)).

## Versions and contracts

- [versions/README.md](versions/README.md) — the four version axes and the rules for answering.
- [versions/capture-a-deployment.md](versions/capture-a-deployment.md) — read chart, image, and API versions from a running cluster.
- [versions/released-inventory.md](versions/released-inventory.md) — NGC inventory for Developer Guide 1.3.0, the previous release, and upgrade boundaries.
- [versions/source-snapshot.md](versions/source-snapshot.md) — component versions in this repository and how to re-read them.
- [versions/api-contracts.md](versions/api-contracts.md) — wire packages, document versions, vendored proto pins.

## The stack

- [stack/overview.md](stack/overview.md) — components, request path, terms.
- [stack/replaceable-and-modular.md](stack/replaceable-and-modular.md) — reference implementations you can replace; start small and grow.
- [stack/choosing.md](stack/choosing.md) — which services for which outcome; install order.
- [stack/security.md](stack/security.md) — cloud security checklist across the stack.

## Deploy

- [deploy/artifact-access.md](deploy/artifact-access.md) — NGC entitlement, chart and image access, pull secrets.
- [deploy/local-process.md](deploy/local-process.md) — optional: Discovery, Storage, and Metadata as local processes, no Kubernetes; [reference services as processes](deploy/local-process/reference-services.md), [building a single local host](deploy/local-process/single-host.md), [validation](deploy/local-process/validate.md). Task procedure: the [local-process-stack](../../local-process-stack/SKILL.md) skill.
- [deploy/local-development.md](deploy/local-development.md) — example (filesystem) stack on a local cluster such as MicroK8s; component dev loops.
- [deploy/production-stack.md](deploy/production-stack.md) — S3 or Azure production stack end to end.
- [deploy/discovery.md](deploy/discovery.md) — Auto Discovery operator and the legacy chart.
- [deploy/notifications.md](deploy/notifications.md) — RabbitMQ, Event Aggregation, Event Consumer.
- [deploy/permissions-and-auth.md](deploy/permissions-and-auth.md) — Envoy auth extension, Permission Service, Permission UI, OIDC.
- [deploy/userinfo.md](deploy/userinfo.md) — Microsoft Graph User Info Service.
- [deploy/thumbnails.md](deploy/thumbnails.md) — Thumbnail Service and workers.
- [deploy/live-edit.md](deploy/live-edit.md) — Live Edit Service (released chart; source not in this repo).
- [deploy/navigator.md](deploy/navigator.md) — Storage Navigator.
- [deploy/script-templates.md](deploy/script-templates.md) — repeatable scripts and values layout.
- [deploy/building-from-source.md](deploy/building-from-source.md) — building each component's images and binaries.

## APIs

- [apis/storage.md](apis/storage.md), [apis/metadata.md](apis/metadata.md), [apis/discovery.md](apis/discovery.md), [apis/notifications.md](apis/notifications.md), [apis/permissions.md](apis/permissions.md), [apis/userinfo.md](apis/userinfo.md), [apis/thumbnails.md](apis/thumbnails.md), [apis/live-edit.md](apis/live-edit.md).

## Develop

- [develop/custom-storage-adapter.md](develop/custom-storage-adapter.md) — implement the Storage API for your backend.
- [develop/add-your-service.md](develop/add-your-service.md) — bring your own service into a stack.
- [develop/custom-notifications-adapter.md](develop/custom-notifications-adapter.md), [develop/custom-permissions-adapter.md](develop/custom-permissions-adapter.md), [develop/custom-thumbnail-implementation.md](develop/custom-thumbnail-implementation.md).

## Test

- [test/conformance.md](test/conformance.md) — every conformance suite and smoke test in the repo.
- [test/integration-tests.md](test/integration-tests.md) — released `storage-api-integration-tests` chart.

## Operate

- [operate/upgrades.md](operate/upgrades.md), [operate/monitoring.md](operate/monitoring.md), [operate/runbooks.md](operate/runbooks.md), [operate/scalability.md](operate/scalability.md), [operate/migration.md](operate/migration.md), [operate/notifications.md](operate/notifications.md), [operate/live-edit.md](operate/live-edit.md), [operate/known-issues.md](operate/known-issues.md).

## Use

- [use/usd-workflows.md](use/usd-workflows.md) — Kit, Client Library, and Navigator for end users.
- [use/companion-products.md](use/companion-products.md) — caches, ovpackage, ovstorage, ovlive.
- [use/related-experts.md](use/related-experts.md) — the ovstorage, ovlive, ovpackage, and usd-caches experts: what each knows and when to hand off.

## Component maps

- [components/README.md](components/README.md) — one map per component into its docs, chart, contracts, changelog, and tests.
