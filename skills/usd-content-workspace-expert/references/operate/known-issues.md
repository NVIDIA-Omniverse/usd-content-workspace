# Known issues and limitations

> **Applies to:** the Developer Guide 1.3.0 release and this repo's snapshot · **Released:** known issues in the [published Developer Guide 1.3.0](https://docs.omniverse.nvidia.com/ovstorage/ovstorage-guide/latest/index.html) · **This repo's snapshot:** checked against component docs and changelogs cited per item · **Sources:** the [source skill package](../../NOTICE.md); this repo's thumbnail, storage, notification and discovery docs; ovlive `client/source/live/protos/PROVENANCE.md`

Each item says whether it comes from the released guide, from this snapshot, or both. Confirm the caller's versions first ([capture-a-deployment](../versions/capture-a-deployment.md)). Incident procedures are in [runbooks.md](runbooks.md).

## Storage

- **Filesystem example persistence (released).** Start the reference service with the explicit backend flag, `python -m local_filesystem_service filesystem --static-dir /data/storage`, on a persistently mounted path. In the released guide, `FILESERVICE_STATIC_DIR` alone could leave files in temporary storage. This snapshot's [filesystem example README](../../../../storage/api/filesystem_example/README.md) documents both the flag and the variable; the flag is safe in both. See [deploy/local-development.md](../deploy/local-development.md).
- **Stale reads after out-of-band writes (both).** Writes that bypass the Storage Service leave cached content and listings stale. Configure a named subscriber under `config.storageEvents` and cache invalidation; with several replicas use the shared Redis/Valkey L2 tier, or lower TTLs, disable caches, or read explicit versions. Replicas on one queue compete for events ([scalability.md](scalability.md#storage-service)).
- **URL normalizer on by default (snapshot Unreleased).** Repeated slashes are collapsed and `.`/`..` resolved after percent-decoding, so objects stored under keys with empty or dot segments become unreachable and writes can overwrite the collapsed key. Audit keys before upgrading or set `config.urlNormalizer.enabled: false` ([upgrades.md](upgrades.md#storage-service)).
- **Resource addresses in REST paths must be URL-encoded (released).**
- **S3-compatible endpoints (released).** Basic read and write do not certify a backend; see [runbooks.md](runbooks.md#validate-s3-capabilities).

## Discovery

- **Stale catalog route in guide runbooks (released).** Some guide runbooks probe `/v2alpha/services`, sometimes on Storage port 8012. The operator's catalog is `GET /api/v2/services` with `"schema-version": 2`.
- **Static nginx examples (released).** Some guide examples describe the legacy static Discovery chart; its values schema is not interchangeable with the operator chart ([deploy/discovery.md](../deploy/discovery.md)).
- **Auto Discovery version (released).** The NGC collection has 1.1.1 while the guide changelog lists 1.1.0; use the acquired archive's values.

## Notifications

- **RabbitMQ version (released).** The release pins `rabbitmq:4.1.3-debian-12-r1` while notification operations recommend 4.1.5 or newer for a policy-repair defect. Do not silently change the baseline; a broker upgrade needs its own validation ([operate/notifications.md](notifications.md)).
- **Durable queues reset on Consumer 1.11.0 (both).** Queues created by earlier consumer versions are unreachable after the upgrade; clients recreate them. Queue ids share one deployment-wide namespace and expire after 24 h without a consumer by default ([upgrades.md](upgrades.md#notifications)).

## Thumbnail Service

- **Asynchronous only (snapshot).** Acceptance (`202`) does not prove generation; there is no task or status API. Discover results by reading associations ([apis/thumbnails.md](../apis/thumbnails.md)).
- **Image formats only (both).** `IMAGE_TO_IMAGE` is the only generator; video and USD processors are rejected. SVG, HEIC/HEIF and AVIF sources are skipped. WebP output is lossless and ignores `quality`.
- **Worker image coordinate (released).** The chart references `nvcr.io/nvidia/omniverse/thumbnail-task-worker`, which the release inventory does not list separately; the source package saw a 403 on its metadata on 2026-10-07. Verify customer access to both images before promising generation ([deploy/artifact-access.md](../deploy/artifact-access.md)).
- **Publish without a bound queue drops work (both).** The orchestrator's publish succeeds even when WorkerEventQueue does not exist; deploy and verify the worker first ([deploy/thumbnails.md](../deploy/thumbnails.md#ordering-worker-queue-before-orchestrator-publishing)).
- **Prefix drift is silent (both).** Orchestrator and worker derive queue ids from `resourcePrefixes`; if they disagree, the worker binds filters that match nothing and sits idle.
- **Recursion guard (both).** Only template-shape matching is active; the writer-principal guard is inert until Storage propagates it. A template without a distinctive shape can produce thumbnails of thumbnails.
- **Plaintext internal transport (snapshot).** Orchestrator and worker refuse `https://` dependency endpoints; service tokens cross the pod network unencrypted. Restrict with NetworkPolicy.
- **Conformance leftovers (both).** The conformance suite leaves generated thumbnails behind and, against a deployment that does not enforce the 1000-specification bound, can make its source file's metadata unreadable. Use disposable sources.
- **Association reads capped at 1000 (both).** No continuation token; narrow with filters.
- **Stale component docs (snapshot).** The service README says gRPC has no `GetThumbnails` (the proto defines it); `DEPLOYMENT_ARCHITECTURE.md` describes the retired Python task queue; the cutover runbook names the pre-merge worker chart. See [components/thumbnail-service.md](../components/thumbnail-service.md#stale-documents).

## Live Edit

- **No read-only participants (released 0.1.1).** Read and write RPCs use the same Storage read/stat check, so anyone who can read a base asset can modify its live sessions. Restrict base-asset read access to intended editors.
- **Reload after recovery (released 0.1.1).** After a checkpoint restore, clients must reload layers from empty state; cached prims can survive otherwise. Keep `layer.catchup_snapshot_enabled: true`. Checkpoints are asynchronous; the 60 s default interval is not a recovery-point bound.
- **Readiness coupling (released 0.1.1).** Enabling `serviceIdentity.accessTokenFile` turns on a service-token readiness check even with checkpoints disabled; keep a valid Storage-audience token at the checkpoint credentials path.
- **Client credentials (released 0.1.1).** `serviceIdentity.clientCredentials` does not acquire the service token; use the token-file provider ([deploy/live-edit.md](../deploy/live-edit.md)).
- **Saturation behavior (released).** The guide says excess subscriptions wait for capacity; do not describe it as immediate refusal.
- **Proto ahead of the service (snapshot of ovlive).** The vendored LiveService proto in ovlive carries operation IDs, chained writes and keepalive fields the 0.1.1 service may not implement; detect support with the `Ping` capability flags ([apis/live-edit.md](../apis/live-edit.md)).

## Clients and compatibility

- **Kit compatibility table (released).** The guide's table covers Kit 109.x, 110.1 and 110.2 with usd-content-workspace releases 1.0–1.2; it does not establish support for the 1.3.0 inventory. Obtain compatibility evidence before certifying.
- **Client Library version (released).** The docs manifest pins Client Library 2.72; examples in this tree use 2.74 APIs. Check the version loaded in the actual Kit process ([components/client-library.md](../components/client-library.md)).
- **Content Cache labels (released).** An upstream note uses "Content Cache 3.x/4.x" for an HTTP-to-HTTPS issue with Kit 110.1. Those are not container pins (the current OVCC chart is 4.0.4 with a `usd-content-cache:3.0.2` image override). Apply the `NO_PROXY=".svc.cluster.local"` workaround only for an affected combination; cache deployment belongs to the usd-caches repository ([use/companion-products.md](../use/companion-products.md)).
- **Migration tool coordinate (released).** The inventory names `migration-tool:0.1.0`; published examples use `ov-nucleus-migration:0.1.0` ([migration.md](migration.md)).
