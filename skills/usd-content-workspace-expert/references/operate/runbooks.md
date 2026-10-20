# Runbooks: recover a degraded deployment

> **Applies to:** a deployed stack that worked and has degraded · **Released:** Developer Guide 1.3.0 inventory (operator runbooks in the [published guide](https://docs.omniverse.nvidia.com/ovstorage/ovstorage-guide/latest/index.html)) · **This repo's snapshot:** checked against Storage Service 1.2.3 + Unreleased, Discovery operator 1.2.0 (Cargo), Thumbnail Service 0.1.0 · **Sources:** [operator README](../../../../discovery/reference/discovery-service/operator/README.md), [storage `SMOKE_TESTS.md`](../../../../storage/reference/storage-service/SMOKE_TESTS.md), [storage-metadata.rst](../../../../storage/reference/storage-service/docs/helm/storage-metadata.rst), [thumbnail chart README](../../../../thumbnails/reference/thumbnail-service/helm/thumbnail-service/README.md), [thumbnail cutover runbook](../../../../thumbnails/reference/thumbnail-service/docs/cutover-runbook.md); adapted from the [source skill package](../../NOTICE.md)

First-deployment limitations are in [known-issues.md](known-issues.md); release validation in [test/integration-tests.md](../test/integration-tests.md); upgrades and rollback in [upgrades.md](upgrades.md). Notification queue incidents: [operate/notifications.md](notifications.md). Live Edit: [operate/live-edit.md](live-edit.md#troubleshooting).

## First response: keep the evidence

Record the failing operation, resource address, time, affected clients and recent changes. Read pod status, restarts, readiness events and logs before restarting anything. Compare the same call through ingress and from inside the cluster; separate a failed capability from a failed service.

```bash
NS=<namespace>
kubectl -n "$NS" get pods,svc,endpoints
kubectl -n "$NS" describe pod <failing-pod>
kubectl -n "$NS" logs <failing-pod> --since=30m
kubectl -n "$NS" get events --sort-by=.lastTimestamp
```

| Symptom | Go to |
| --- | --- |
| Clients cannot find services | [Discovery](#inspect-discovery) |
| Image pull failures | [ImagePullBackOff](#imagepullbackoff) |
| Stat or read fails or is slow | [Read path](#isolate-the-read-path) |
| Reads work but remote changes stay stale | [Notifications](#trace-notifications) |
| Metadata or history fails; access denied broadly; identity lookups fail | [Databases](#recover-database-dependencies) |
| Failures after rotation or at regular intervals | [Credentials](#repair-credential-delivery) |
| External calls fail, internal work; certificate errors | [TLS](#restore-tls) |
| DNS errors, refusals, timeouts, throttling | [Connectivity and capacity](#check-connectivity-and-capacity) |
| Non-AWS S3 accepts auth but rejects some operations | [S3 capabilities](#validate-s3-capabilities) |
| Thumbnails missing | [Thumbnails](#thumbnails-stopped-appearing) |
| A new release or values change fails | [upgrades.md](upgrades.md#roll-back-safely) |

## Inspect Discovery

The operator watches annotated Services and serves `GET /api/v2/services` on its internal port (8080) and external port (8081); require `"schema-version": 2`. Compare advertised entries with Services, ready endpoints and operator logs.

- The Service needs `discovery.omniverse.nvidia.com/enabled: "true"`.
- gRPC port: the `discovery.omniverse.nvidia.com/grpc-port` annotation, else a port named `grpc`, else the service is skipped. REST: a port named `http` or `rest`.
- `visibility` (`internal`, `external`, `both`; default `both`) and `external-rest-endpoint` / `external-grpc-endpoint` override what is published; otherwise external URLs come from Contour HTTPProxies.
- The operator calls `CapabilitiesService/ListServices` unauthenticated; a service that requires a token there is skipped.
- Reconciliation runs on Service/HTTPProxy changes and every 30 s by default.

Live Edit advertises on `50001`; the Thumbnail Service on its `grpc` port (`50051`). An advertised URL is not proof of readiness, access or client reachability; test it from the client network and refresh any cached client catalog.

Some released guide runbooks still probe `/v2alpha/services`, sometimes on Storage port 8012. That is not the operator's catalog route; use the Discovery Service and `/api/v2/services` ([apis/discovery.md](../apis/discovery.md)).

## ImagePullBackOff

Compare the manifest's full repository and tag or digest with the registry. Check the pull Secret exists in the pod's namespace and targets `nvcr.io` (not the catalog or Helm host), without printing its contents. Verify NGC entitlement for the exact artifact; an employee account's access does not prove a customer's. For locally imported MicroK8s images, `microk8s ctr images ls` on the node, exact name and tag, and `imagePullPolicy: Never`; imports are per node. See [deploy/artifact-access.md](../deploy/artifact-access.md).

## Isolate the read path

1. Stat a known resource with the caller's token, then read it. A successful stat narrows the fault to transfer.
2. Correlate Storage logs with backend denial, timeout or throttling; distinguish latency that grows with object size from constant per-request delay.
3. Check metadata-cache health and invalidation; a cache outage sends traffic to origin, while stale data with successful requests points at notifications.
4. For presigned redirects, test the object-store host from the client's network. Keep bearer tokens and presigned URLs out of logs.

```bash
curl --head -sS -o /dev/null -w '%{http_code}\n' \
  -H "Authorization: Bearer ${TOKEN}" \
  "https://<storage-host>/v1alpha/fileobject/by-address/<url-encoded-address>"
```

Restore the failing dependency, reduce concurrency under throttling, and verify stat, small and large reads, latency and cache behavior. Storage configuration: [deploy/production-stack.md](../deploy/production-stack.md); contract: [apis/storage.md](../apis/storage.md); throttling and load shedding: [scalability.md](scalability.md#storage-service).

## Trace notifications

Chain: object store → backend queue (SQS or Service Bus) → Storage event subscriber (`config.storageEvents`) → Event Aggregation → RabbitMQ → Event Consumer → client.

- Measure source-queue active and dead-letter counts and growth before scaling.
- Scope subscriptions to required containers; check TTL and dead-letter-on-expiry so backlog does not become silent loss.
- Give independent environments their own queues; consumers sharing one queue each get a subset.
- Each Storage replica runs `workers` consumers per subscription, so `N` workers × `R` replicas can reach `N·R` concurrent consumers ([scalability.rst](../../../../storage/reference/storage-service/docs/helm/scalability.rst)).
- Delivery is at least once; make consumers idempotent on resource address plus version or ETag.
- Dead-lettered work does not drain itself; re-drive or discard deliberately.

Details: [operate/notifications.md](notifications.md).

## Recover database dependencies

Decide which database owns the failed capability: Storage metadata (PostgreSQL or CosmosDB), the Permission Service's PostgreSQL, or User Info's dependencies. Read connection, authentication and pool errors before touching pods. Check primary and volume status, DNS and TCP from the service, credential sync, and connection counts. Recover the database first; restarting consumers during the outage discards evidence and causes crash loops. Never delete and recreate a persistent database as a restart.

Storage applies PostgreSQL migrations in a bootstrap init container (`STORAGE_SERVICE_BOOTSTRAP_ONLY=true`) before replicas start; the token provisioner runs as a native sidecar and needs Kubernetes 1.29+. A pod stuck in `Init` after an upgrade usually means the bootstrap container cannot reach or authenticate to the database, or is waiting on the migration lock ("database migration is still running" every 10 s).

Verify the specific operation and normal connection use, not just pod readiness. See [deploy/permissions-and-auth.md](../deploy/permissions-and-auth.md) and [deploy/userinfo.md](../deploy/userinfo.md).

## Repair credential delivery

Identify the rejected credential and the action; expiry and missing grants need different fixes. Trace it from issuer or vault through ExternalSecret, Kubernetes Secret, mount or environment variable, to the process.

- Confirm ExternalSecret sync without printing data.
- Environment values are fixed for a pod's lifetime; rotate in the deployment source, then roll the Deployment.
- Mounted files help only if the process rereads them. Token files (Thumbnail `serviceIdentity.accessTokenFile.pollInterval`, Live Edit `pollIntervalSecs`) are reread; check the provisioner's health and file freshness.
- Thumbnail Service and worker exit if no bearer arrives within 60 s of start (`TOKEN_READY_TIMEOUT`); a lagging sidecar shows up as CrashLoopBackOff.
- For workload identity, confirm the federated credential and ServiceAccount name still match.
- Fix the issuer or provider before discarding the last working secret; never embed a credential in a running Deployment as the lasting fix.

For S3 on Storage 1.2.x use the nested `credentials` object; flat AWS keys were removed in 1.2.0 ([migration.rst](../../../../storage/reference/storage-service/docs/helm/migration.rst)).

## Restore TLS

Inspect the certificate actually served for the requested SNI; distinguish expiry, not-yet-valid, missing SAN and unknown CA. Compare external and internal paths. With cert-manager, read Certificate, CertificateRequest, Order and Challenge status before forcing renewal. For OIDC key rollover, confirm the JWKS is reachable before restarting a consumer that cannot reload it. Never bypass verification as a fix. Keep changes in the release workflow.

The Thumbnail Service and worker speak plaintext HTTP/2 to Storage and Notifications and refuse `https://` endpoints; TLS for them terminates only at the Contour ingress.

## Check connectivity and capacity

Test DNS, TCP, TLS and authentication from the affected pod's network, and compare another pod, namespace or node to separate NetworkPolicy from routing. Minimal images may lack shells or `curl`; use an approved debug pod with the same network placement. A hard refusal and a black-hole timeout behave differently: timeouts hold connections. Under explicit throttling, reduce source concurrency before adding load. Do not restart healthy adapters because an upstream provider is down.

## Validate S3 capabilities

Basic read and write do not certify an S3-compatible endpoint. Configure addressing style and versioning explicitly. Exercise stat, list, versioned read, conditional write, copy, move, multipart and presigned transfer; confirm preconditions are enforced. Treat ETags as opaque. Known gaps include bodyless copy returning 411, partial version listings, key-length limits and checksum or trailer support. Anonymous credentials are read-only. Disable dependent features deliberately when unsupported.

## Thumbnails stopped appearing

| Observation | Likely cause | Action |
| --- | --- | --- |
| No errors anywhere; StorageEventQueue busy; WorkerEventQueue at zero deliveries | Worker and orchestrator disagree on `resourcePrefixes` | Compare digests of `thumbnail-service-storage-<sha256>` and `thumbnail-service-worker-<sha256>`; redeploy both with one prefix set |
| CREATEs acknowledged, nothing generated, no worker activity | WorkerEventQueue missing when the orchestrator published (publish succeeds with no bound queue) | Confirm the queue exists and the worker is Ready, then backfill with `scripts/create-missing-thumbnails.sh` |
| Worker not Ready | No consume stream: Event Consumer unreachable, missing `event_permissions` entry for `omni.thumbnail.generate`, or missing `consume-durable-queues` grant | Read worker logs; fix the Notification deployment or grants |
| Startup logs `PermissionDenied` on `CreateDurableQueue` | Identity lacks `create-durable-queues` | Grant it, or pre-create the queue |
| Pods CrashLoopBackOff early | No bearer within 60 s, both or neither identity mode, unparsable template, or an `https://` endpoint | Read the first log lines |
| `thumbnail_worker_failures_total{class="decode"}` or permanent failures | Unsupported, oversize or corrupt sources; over `maxTransformPeakBytes` | Expected for bad inputs; raise budget and memory limit together only if needed |
| `retryable_failure` and `nacks_total` climbing | Storage or associate call failing | Check Storage and the orchestrator's gRPC Service |
| `thumbnail_worker_unacknowledged_deliveries` rising | Retryables accumulating; queue nearing max length | Fix the dependency; inspect the DLQ with `ManageDeadLetteredEvents` |
| Thumbnails of thumbnails | Template shape no longer recognized | Restore a template with a distinctive shape; the writer-principal guard is inert |
| A new source's thumbnails missing only for SVG/HEIC/AVIF | Not eligible | Expected |

Per-item task status no longer exists; find an individual failure in worker logs by destination address (each permanent rejection is logged at error level before acknowledgement).

```bash
kubectl -n "$NS" logs deploy/thumbnail-service -c thumbnail-service --tail=200
kubectl -n "$NS" logs deploy/thumbnail-service-thumbnail-task-worker -c thumbnail-worker --tail=200
```

Backfill with the script's documented options (header of `scripts/create-missing-thumbnails.sh`); run `--dry-run` first:

```bash
thumbnails/reference/thumbnail-service/scripts/create-missing-thumbnails.sh \
  --folder "<watched prefix>/" --token "${TOKEN}" \
  --service http://localhost:3000 --storage <storage-grpc-host:port> --dry-run --limit 10
```

The script's usage example obtains the token from `scripts-notifications/get_jwt_token.sh`, which is not in this snapshot. Use a short-lived token from the caller's identity and keep it out of shell history.
