# Deploy the Thumbnail Service

> **Applies to:** Thumbnail Service (orchestrator `thumbnail-service` and conversion worker `thumbnail-task-worker`, one Helm chart) · **Released:** chart and `thumbnail-service` image 0.1.0 (usd-content-workspace Developer Guide 1.3.0 / NGC inventory); the worker image is not named separately in that inventory · **This repo's snapshot:** Cargo `[workspace.package] version = "0.1.0"` (`SOURCE_VERSIONS.md`); `Chart.yaml` holds the `0.0.0` placeholder · **Sources:** [`helm/thumbnail-service/values.yaml`](../../../../thumbnails/reference/thumbnail-service/helm/thumbnail-service/values.yaml), [chart README](../../../../thumbnails/reference/thumbnail-service/helm/thumbnail-service/README.md), [service README](../../../../thumbnails/reference/thumbnail-service/README.md), [`docs/sdd.md`](../../../../thumbnails/reference/thumbnail-service/docs/sdd.md), [`helm/DEPLOYMENT_ARCHITECTURE.md`](../../../../thumbnails/reference/thumbnail-service/helm/DEPLOYMENT_ARCHITECTURE.md), [published service docs 0.1.0](https://docs.omniverse.nvidia.com/ovstorage/thumbnail-service/0.1.0/index.html); adapted from the [source skill package](../../NOTICE.md)

The Thumbnail Service and its worker are released under Apache-2.0. Confirm the version the caller runs with [capture-a-deployment](../versions/capture-a-deployment.md) before applying values from this page, and read the matching chart's `values.yaml` and README before upgrading ([upgrades](../operate/upgrades.md)).

API contract: [apis/thumbnails.md](../apis/thumbnails.md). Operations: [monitoring](../operate/monitoring.md), [runbooks](../operate/runbooks.md#thumbnails-stopped-appearing). Component map: [components/thumbnail-service.md](../components/thumbnail-service.md).

## What one release deploys

One chart renders two independent Deployments.

| | Orchestrator | Worker |
| --- | --- | --- |
| Deployment name | `<release>-thumbnail-service` (or `<release>` when the release name contains `thumbnail-service`) | `<release>-thumbnail-task-worker` |
| Container / binary | `thumbnail-service`, `/app/server` | `thumbnail-worker`, `/app/task-worker` |
| Image (default repository) | `nvcr.io/nvidia/omniverse/thumbnail-service` | `nvcr.io/nvidia/omniverse/thumbnail-task-worker` |
| Values | `thumbnailService.*` | `thumbnailWorker.*` |
| Serves | REST `3000`, gRPC `50051` (Service named by `service.nameOverride`, default `thumbnail-service`) | nothing; `/healthz`, `/readyz`, `/metrics` on `workerRuntime.healthPort` (`8080`) |
| Durable queue | StorageEventQueue: `omni.storage.created`, `omni.storage.deleted` | WorkerEventQueue: `omni.thumbnail.generate` |
| Acknowledgement means | generate work was published | thumbnail converted, uploaded, and associated |
| Scales on | request volume (HPA off by default) | conversion CPU (HPA on, 2–20, 70%) |

Flow: a storage CREATE (or a `CreateThumbnails` call) reaches the orchestrator, which checks eligibility, resolves one destination per size from `thumbnailPathTemplate`, and publishes one `omni.thumbnail.generate` event per thumbnail. The worker converts, writes the result through Storage gRPC, and calls `AssociateThumbnail` on the orchestrator's in-cluster gRPC Service. The orchestrator then publishes `omni.thumbnail.associated`. Generation is asynchronous: a `202` or an acknowledged CREATE does not prove a thumbnail exists.

The snapshot no longer uses the retired Python `thumbnail-task-queue` (Celery, RabbitMQ vhost, PostgreSQL). The second half of `DEPLOYMENT_ARCHITECTURE.md` still describes it; ignore that half.

## Prerequisites

| Dependency | Value key | Default | Notes |
| --- | --- | --- | --- |
| Storage Service gRPC | `storage.endpoint` | `storage-service:8011` | Shared by both Deployments; no per-component override. See [deploy/production-stack.md](production-stack.md). |
| Event Consumer gRPC | `notifications.eventConsumer.endpoint` | `event-consumer-service:50051` | Required when either consumer is enabled. See [deploy/notifications.md](notifications.md). |
| Event Aggregation gRPC | `notifications.eventAggregation.endpoint` | `event-aggregation-service:50051` | Used when `notifications.eventAggregation.enabled` (default `true`). |
| Watched container roots | `notifications.eventConsumer.resourcePrefixes` | `[]` | Required, non-empty. Container roots, not the thumbnail output path. |
| Destination template | `thumbnailPathTemplate` | `""` | Required; validated at startup, not at render time. |
| Service identity | `serviceIdentity.*` | both modes `false` | Exactly one mode must be enabled. |
| Discovery | Service annotation | `discovery.omniverse.nvidia.com/enabled: "true"` set by the chart | See [deploy/discovery.md](discovery.md). |

Outbound connections are plaintext HTTP/2. An endpoint beginning `https://` is refused at startup because this build links `tonic` without TLS. Service-account bearer tokens therefore cross the pod network unencrypted. Keep all three dependencies on cluster-internal Service addresses, restrict them with NetworkPolicy, and treat the cluster network as the trust boundary (chart README, "Outbound TLS posture"). Do not point these endpoints at external hosts.

### Notification-side grants (do these first)

The thumbnail pipeline needs changes on the Notification deployment and on the shared service principal before the first rollout (chart README, "Prerequisites before the first rollout"):

1. An `event_permissions` entry for `omni.thumbnail.generate` in the file named by the Notification services' `OMNI_EVENTS_PERMISSIONS_YAML_FILE_PATH`, plus the matching policy on `EventResourceId`. Durable consumption of a non-storage event type uses this fine-grained check. This is a Notification deployment change; see [deploy/notifications.md](notifications.md).
2. Grants for the shared service principal (orchestrator and worker run as one identity): `event-aggregation-service:publish-event` on `EventType:omni.thumbnail.generate`, `event-consumer-service:create-durable-queues`, `event-consumer-service:consume-durable-queues`. See [deploy/permissions-and-auth.md](permissions-and-auth.md).
3. Storage access for the same principal: read on watched sources, write and delete on the destination prefix, and metadata write on sources and thumbnails (associations are stored as Storage metadata on both sides).

If the identity lacks `create-durable-queues`, `CreateDurableQueue` returns `PermissionDenied`; the service logs a warning and attaches to a queue that must already exist (`docs/sdd.md`, "Error semantics").

## Values

Runtime settings live at the chart root; `thumbnailService` and `thumbnailWorker` hold only workload settings. There is no `common:` wrapper.

### Destination template

`thumbnailPathTemplate` must render an absolute storage address and must identify the source with `{resource_address_md5}`, or with both `{resource_path}` and `{resource_filename}`. Variables: `{resource_path}` (source directory, no trailing slash), `{resource_filename}`, `{resource_address_md5}`, `{resource_identity_md5}`, `{width}`, `{height}`, `{output_encoding}` (lowercase), `{quality}`. Substitution is verbatim; write every `/` yourself.

```yaml
thumbnailPathTemplate: "https://<account>.blob.core.windows.net/<container>/thumbnails/{resource_address_md5}/{resource_identity_md5}/{width}x{height}/q{quality}/{resource_filename}.auto.{output_encoding}"
```

Including `{resource_identity_md5}` gives each source version its own thumbnail; omitting it makes a new version overwrite the old thumbnail. Clients find thumbnails by reading associations, never by evaluating the template.

Recursion prevention: the only active guard is template-shape matching (CREATE events whose address matches the rendered template shape are skipped). The writer-principal check (`GENERATED_THUMBNAIL_PRINCIPAL_IDENTITY`, injectable only through `thumbnailService.extraVars`) is inert because Storage does not yet propagate the writer principal ([ADR 0001](../../../../thumbnails/reference/thumbnail-service/docs/adr/0001-thumbnail-storage.md)). Do not weaken the template shape.

### Generation defaults

| Key | Default | Meaning |
| --- | --- | --- |
| `thumbnailSizes` | `256x256,512x512` | Fan-out when a request or event carries no specifications |
| `thumbnailConfig.encodingFormat` | `PNG` | `PNG`, `JPEG`, `GIF`, `WEBP` |
| `thumbnailConfig.transformType` | `FIT` | Padding (transparent for PNG, white for JPEG) rather than cropping |
| `thumbnailConfig.cropStrategy` | `SMART` | Inert while `FIT` |
| `thumbnailConfig.quality` | `"85"` | Used when a specification omits quality |
| `thumbnailConfig.preserveAspectRatio` | `"true"` | Used when a specification omits it |

Eligible source extensions on the CREATE path: `jpg`, `jpeg`, `png`, `gif`, `bmp`, `tiff`, `tif`, `webp`, `ico` (`crates/common/src/source_formats.rs`). SVG, HEIC/HEIF and AVIF are skipped. WebP output is lossless and ignores `quality`.

### Worker runtime and memory

| Key | Default | Meaning |
| --- | --- | --- |
| `workerRuntime.maxConcurrentConversions` | `2` | Conversions in flight per worker pod |
| `workerRuntime.maxTransformPeakBytes` | `1073741824` | Per-conversion memory budget; over-budget inputs fail permanently instead of OOM |
| `workerRuntime.maxSourceBytes` | `67108864` | Largest source downloaded |
| `workerRuntime.maxDecodedPixels` | `64000000` | Largest decoded image |
| `workerRuntime.outboundRequestTimeout` | `30s` | Storage and associate call deadline |
| `workerRuntime.shutdownDrainTimeout` | `30s` | Drain on SIGTERM; `thumbnailWorker.terminationGracePeriodSeconds` (`45`) must exceed it |
| `workerRuntime.redeliveryRecycleInterval` | `60s` | Delay before a retryable failure is negatively acknowledged |
| `thumbnailWorker.resources` | requests `1300m`/`750Mi`, limits `2500m`/`2304Mi` | Render fails unless `limits.memory ≥ maxConcurrentConversions × maxTransformPeakBytes + 200Mi` |

Sizing guidance is in [operate/scalability.md](../operate/scalability.md#thumbnail-service).

### Service identity

Exactly one mode; the chart (`service.validateServiceIdentity`) and the binary both refuse zero or two.

```yaml
# Preferred where workload identity exists: a provisioner sidecar writes the token file.
serviceIdentity:
  accessTokenFile:
    enabled: true
    path: /etc/secrets/access-token/token
    pollInterval: 15s
    provisionerSidecar:
      container:
        name: token-provisioner
        image: <token-provisioner-image>:<tag>
        # env/args for your identity provider; keep the chart's small CPU/memory requests
serviceAccount:
  name: thumbnail-service        # one name for both components if a federated credential binds to it
podLabels:
  azure.workload.identity/use: "true"   # only for Azure Workload Identity
```

```yaml
# Client credentials: the Secret must already exist; the chart never creates Secrets.
serviceIdentity:
  clientCredentials:
    enabled: true
    clientId: <client-id>
    clientScope: <api-audience>/.default
    openIdConfigurationUri: https://<idp>/.well-known/openid-configuration
    clientSecretRef:
      name: thumbnail-oauth         # existing Secret
      key: CLIENT_SECRET
```

Both binaries abort startup if no bearer is available within 60 seconds (`TOKEN_READY_TIMEOUT`); prefer a native sidecar (init container with `restartPolicy: Always`) so the token file exists before the app starts. The startup probes allow about 125 seconds.

### Ingress and CORS

`httpProxy.enabled: true` creates Contour HTTPProxies: REST at `<host>.<domain>` and gRPC at `grpc.<host>.<domain>` (h2c to the pod). `httpProxy.tls.enabled` defaults to `false`; enable it, and optionally `httpProxy.tls.certManager`. Set `httpProxy.authExtension.enabled: true` to put the routes behind the Envoy auth extension ([deploy/permissions-and-auth.md](permissions-and-auth.md)). The default `corsPolicy` allows origin `*` and headers `*`; narrow `allowOrigin` to the real UI origins. Default route timeouts are `10s`.

The chart README's HTTPProxy example nests `httpProxy` under `thumbnailService`; that is the pre-migration shape. In this snapshot `httpProxy` is a root key.

## Install

Released chart (verify both images are entitled before promising generation; see [deploy/artifact-access.md](artifact-access.md)):

```bash
helm fetch https://helm.ngc.nvidia.com/nvidia/omniverse/charts/thumbnail-service-0.1.0.tgz \
  --username='$oauthtoken' --password="${NGC_API_KEY}"
tar -xzf thumbnail-service-0.1.0.tgz
helm lint ./thumbnail-service -f thumbnail-values.yaml
helm template thumbnail-service ./thumbnail-service -n <namespace> -f thumbnail-values.yaml > rendered.yaml
```

Read `rendered.yaml`: both images and tags, the Service annotations, the token sidecar, and the absence of any Secret. Set `image.tag` explicitly (`"0.1.0"` for the release); it applies to both images. Building images from this repo: [deploy/building-from-source.md](building-from-source.md) (`docker/Dockerfile` targets `server` and `task-worker`, build context = repo root).

### Ordering: worker queue before orchestrator publishing

`PublishEvent` succeeds even when no queue is bound to `omni.thumbnail.generate`. An orchestrator that publishes before WorkerEventQueue exists acknowledges every CREATE and silently drops the work. A single `helm upgrade` starts both Deployments without ordering them. For a first install or a cutover:

1. Install with the orchestrator unable to publish, for example `notifications.eventConsumer.storageEventQueue.enabled: false` (stops CREATE-driven work) and no API clients yet.
2. Confirm the worker is Ready (`/readyz` is Ready only with an established consume stream) and that WorkerEventQueue `thumbnail-service-worker-<sha256>` exists.
3. Enable StorageEventQueue and upgrade.

```bash
helm upgrade --install thumbnail-service ./thumbnail-service -n <namespace> -f thumbnail-values.yaml
kubectl -n <namespace> rollout status deployment/thumbnail-service
kubectl -n <namespace> rollout status deployment/thumbnail-service-thumbnail-task-worker
```

### Queue identity

Each binary derives its queue id from the sorted, de-duplicated `resourcePrefixes`: `thumbnail-service-storage-<sha256>` and `thumbnail-service-worker-<sha256>`. Both share one digest when they agree; differing digests mean one Deployment has stale configuration. Changing `resourcePrefixes` moves both components to new queues; work left on the old queues is not migrated. Treat a prefix change as a cutover and keep the old queues until drained.

## Discovery

The chart annotates the Service with `discovery.omniverse.nvidia.com/enabled: "true"` and names its ports `rest` and `grpc`, so the operator needs no `grpc-port` annotation. The orchestrator answers `CapabilitiesService/ListServices` without authentication, advertising service type `thumbnail`, gRPC `nvidia.omniverse.thumbnail.v1alpha.ThumbnailService`, and REST endpoint `thumbnail` at version `v1alpha`, path `/v1alpha/`. Visibility defaults to the operator's `both`; override with `service.annotations` (`discovery.omniverse.nvidia.com/visibility`, `.../external-rest-endpoint`, `.../external-grpc-endpoint`). Setting `discovery.omniverse.nvidia.com/enabled: "false"` removes the service from the catalog. The snapshot operator also maps it to v1 type `thumbnail` for older clients (`v1Endpoints` in the operator values). Verify `GET /api/v2/services` with `"schema-version": 2`, then call the advertised URLs from the client network.

## Validate the outcome

```bash
kubectl -n <namespace> port-forward svc/thumbnail-service 3000:3000 50051:50051
curl -fsS http://localhost:3000/health/live      # liveness, dependency-free
curl -fsS http://localhost:3000/health           # 503 until gRPC is serving
kubectl -n <namespace> port-forward deploy/thumbnail-service-thumbnail-task-worker 8080:8080
curl -fsS http://localhost:8080/readyz && curl -fsS http://localhost:8080/metrics | grep thumbnail_worker_outcomes_total
```

Then prove the real outcome: upload a small JPEG or PNG into a watched prefix, wait, and read its associations with a caller token. Success is a non-empty `thumbnails` list whose thumbnail address is readable through Storage.

```bash
curl -fsS -G -H "Authorization: Bearer ${TOKEN}" \
  --data-urlencode "association_target_resource_address=<uploaded address>" \
  http://localhost:3000/v1alpha/thumbnails
```

`smokeTest.enabled: true` adds a Job that runs as a Helm `test`, `post-install` and `post-upgrade` hook (associate, read, disassociate on `smokeTest.sourceFile`); a failing smoke test fails the install. It does not prove conversion. The conformance chart is separate: [develop/custom-thumbnail-implementation.md](../develop/custom-thumbnail-implementation.md#certify-with-the-conformance-suite). Conformance runs can leave thumbnails behind; use disposable sources.

## Rollback

Roll back both sides together, keep both durable queues, and drain the worker before scaling it down (`thumbnail_worker_unacknowledged_deliveries` at zero). See [operate/upgrades.md](../operate/upgrades.md#thumbnail-service).
