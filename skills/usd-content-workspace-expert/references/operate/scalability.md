# Scale the stack

> **Applies to:** Storage Service, Thumbnail Service, Live Edit (pointer), Notifications (pointer) · **Released:** Storage 1.2.3, Thumbnail 0.1.0, Live Edit 0.1.1 (Developer Guide 1.3.0) · **This repo's snapshot:** Storage 1.2.3 + Unreleased (`CHANGELOG.md`); Thumbnail 0.1.0 (Cargo) · **Sources:** [`docs/helm/scalability.rst`](../../../../storage/reference/storage-service/docs/helm/scalability.rst), [`request-throttling.rst`](../../../../storage/reference/storage-service/docs/helm/request-throttling.rst), [`storage-backend-caching.rst`](../../../../storage/reference/storage-service/docs/helm/storage-backend-caching.rst), [`redis-client.rst`](../../../../storage/reference/storage-service/docs/helm/redis-client.rst), [thumbnail `values.yaml`](../../../../thumbnails/reference/thumbnail-service/helm/thumbnail-service/values.yaml) and [chart README](../../../../thumbnails/reference/thumbnail-service/helm/thumbnail-service/README.md); adapted from the [source skill package](../../NOTICE.md)

Size from measurements: request rate, object sizes, metadata and enumeration latency, backend limits and memory. A GPU-to-node ratio is not a capacity test. Confirm the running versions and read their `values.yaml` before changing anything ([capture-a-deployment](../versions/capture-a-deployment.md)). Keep changes in the maintained values files; retain requests, disruption budgets and existing autoscalers.

## Storage Service

### Resources

| Key | Default |
| --- | --- |
| `resources.requests.cpu` / `.limits.cpu` | `8` / `8` |
| `resources.requests.memory` / `.limits.memory` | `2Gi` / `4Gi` |
| `resources.requests.ephemeral-storage` / `.limits.ephemeral-storage` | `256Mi` / `1Gi` |

The same resources apply to the bootstrap (migration) container; custom init containers and sidecars declare their own.

Set `resources.limits.memory` to at least the sum of every enabled cache's capacity plus 1 GiB for request handling: `config.smallObjectCache.capacity`, `config.statCache.capacity`, `config.listCache.capacity`, `config.metadataCache.cacheCapacity`, `config.manifest.cacheCapacity`. Three 1 GiB caches need at least 4 GiB.

### Caches and staleness

Cache TTLs (`config.statCache.timeToLive`, `config.listCache.timeToLive`, `config.smallObjectCache.timeToLive`) are durations; take defaults and units from `storage-backend-caching.rst`, do not invent them. Say which cache is being tuned: stat results, listings, or small-object content. A lower TTL shortens stale windows and raises backend load.

With more than one replica and any cache enabled, enable the shared Redis/Valkey tier and the L2 flag on every enabled cache. It gives replicas a shared L2 and carries invalidation messages between them.

```yaml
config:
  redis:
    enabled: true
    address: "redis://valkey.infra.svc.cluster.local:6379"   # use TLS and credentials per redis-client.rst
  metadataCache:
    l2Enabled: true
  smallObjectCache:
    l2Enabled: true
  statCache:
    l2Enabled: true
  listCache:
    l2Enabled: true
  manifest:
    l2Enabled: true
```

Without the shared tier, replicas can serve stale latest-version reads. Then lower TTLs, disable the affected caches, or have clients read explicit versions. Validate across replicas and after out-of-band writes.

### Event subscribers

`config.storageEvents` is a map keyed by subscriber name (several of the same kind are allowed); obsolete fixed paths like `config.storageEvents.sqs.enabled` do not exist. Every replica starts every subscriber, and `workers` applies per replica: `N` workers on `R` replicas can run `N·R` consumers. Reduce `workers` as replicas grow:

```yaml
config:
  storageEvents:
    s3-events:
      sqs:
        workers: 2
    azure-blob-events:
      azureServiceBus:
        workers: 2
```

Replicas on one queue compete for messages; each does not see every event, so storage events alone do not give cross-pod cache invalidation. Give independent deployments separate backend queues.

### Throttling and load shedding

| Key | Default | Effect |
| --- | --- | --- |
| `config.ingressThrottle.enabled` / `.maxConcurrency` | `true` / `8192` | Concurrent inbound requests (REST and gRPC) |
| `config.ingressLoadShed.enabled` | `true` | Reject at the limit: REST `429`, gRPC `RESOURCE_EXHAUSTED` |
| `config.egressThrottle.enabled` / `.maxConcurrency` | `false` / `4096` | Concurrent backend requests |

Egress throttling can queue requests indefinitely and grow memory; keep ingress throttling on whenever egress throttling is on. Clients should back off on `429` and `RESOURCE_EXHAUSTED`.

### Replicas

Change `replicaCount` in maintained values after measuring. Re-check worker totals, cache memory, Redis connections and database pool limits whenever the count changes.

## Thumbnail Service

The two Deployments scale independently.

| | Orchestrator | Worker |
| --- | --- | --- |
| Replicas | `thumbnailService.replicaCount: 1` | `thumbnailWorker.replicaCount: 2` (superseded by the HPA) |
| HPA | `thumbnailService.autoscaling.enabled: false` (1–10, 80% CPU) | `thumbnailWorker.autoscaling.enabled: true` (2–20, 70% CPU) |
| Resources | requests `250m`/`64Mi`, limits `1`/`256Mi` (API and event filtering only) | requests `1300m`/`750Mi`, limits `2500m`/`2304Mi` |
| Concurrency | `streaming.maxConcurrent: 100` (StorageEventQueue prefetch) | `workerRuntime.maxConcurrentConversions: 2` |

Worker memory rule, enforced at render: `limits.memory ≥ maxConcurrentConversions × maxTransformPeakBytes + ~200 MiB` (2 × 1 GiB + 200 MiB → `2304Mi`). The resample, not the decode, sets the peak: a working buffer of `source_width × new_height × 16 B`. The largest conversion the API allows (64 MP source to 8192×8192) needs about 1744 MiB and is rejected with the default budget; the configured 256 and 512 sizes peak far lower. Raise `maxTransformPeakBytes` and `limits.memory` together only if you must serve the ceiling.

CPU: one conversion slot averages about 0.6 core across download, convert, upload and associate, so two slots draw about 1.2–1.4 cores, which is why the request is `1300m`. The CPU limit sizes the async runtime (`available_parallelism()` rounds the cgroup quota down), so do not lower `limits.cpu` below about 2 cores without testing. Measured throughput was about 600 conversions per minute per worker (chart README, derived from a concurrency-4 load test).

Do not enlarge the token-provisioner sidecar's requests (`10m`/`32Mi`): HPA utilization divides by the sum of container requests, and a larger sidecar request dilutes the worker's CPU signal. The sidecar must have a CPU request or the worker HPA cannot compute utilization.

The values file disagrees with itself about the orchestrator: the `replicaCount` comment says recursion prevention is stateless and replicas are unconstrained, while the `autoscaling.enabled` comment warns that an in-process cache requires a single replica. `DEPLOYMENT_ARCHITECTURE.md` and ADR 0001 support the stateless reading. Scale the orchestrator only on request volume, and verify no thumbnails-of-thumbnails appear after scaling.

Backlog shows as WorkerEventQueue depth and `thumbnail_worker_unacknowledged_deliveries`. Each extra worker adds load on Storage and the orchestrator's associate path; watch Storage latency and `retryable_failure` while scaling out.

## Live Edit

Subscription capacity (`valkey.subscriber_pool_size`, 256 per replica), long-lived streams that stay on their original pod, and Valkey sizing each need their own tests. Defaults and caveats: [operate/live-edit.md](live-edit.md#scaling-and-capacity).

## Notifications

Event Consumer stream packing, admission caps, broker memory and the durable pool are separate sizing decisions. Keep the default autoscaler's REST metrics and do not add a competing HPA. Test reconnects during a rolling update. See [operate/notifications.md](notifications.md).
