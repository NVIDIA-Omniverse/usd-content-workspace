(thumbnail-service-troubleshooting-troubleshooting)=
# Troubleshooting

This page maps the symptoms an operator is most likely to see to their causes and to the log line, metric, or command that confirms each one. It assumes the deployment described in {ref}`Thumbnail Service Configuration <thumbnail-service-deployment-thumbnail-service-configuration>` and the telemetry described in {ref}`Observability <thumbnail-service-observability-observability>`.

(thumbnail-service-troubleshooting-thumbnails-stop-appearing-with-no-errors)=
## Thumbnails stop appearing with no errors

- **Both components report healthy, StorageEventQueue consumes normally, and nothing is generated.** The worker's WorkerEventQueue filters on the same source-address prefixes the service watches. A worker whose `NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES` differs from the service's binds filters that match nothing and sits at zero deliveries — indistinguishable from no eligible files arriving. Run the {ref}`queue id digest check <thumbnail-service-troubleshooting-queue-id-digest-check>`.
- **Generate events publish but the worker never consumes.** WorkerEventQueue must exist **before** the service publishes: `PublishEvent` succeeds whether or not any queue is bound, so a service rolled out ahead of its worker acknowledges storage deliveries for work that is silently dropped. Confirm the worker's `[PROVISION] Durable queue created` or `already existed` line predates the service rollout; see {ref}`Prerequisites before the first rollout <thumbnail-service-deployment-prerequisites-before-the-first-rollout>`.
- **Files are uploaded but never dispatched.** The file may be ineligible. Skips are logged only at `debug` (`Skipping CREATE event` with a `reason`): unsupported extension, a folder-shaped address, or an address that matches the configured `THUMBNAIL_PATH_TEMPLATE` shape — the recursion guard. Raise `observability.logging.level` to `thumbnail_service=debug,info` and watch one upload. See {ref}`Generated-output recursion prevention <thumbnail-service-destination-path-template-generated-output-recursion-prevention>`.
- **The consumer is not running at all.** Check that `notifications.eventConsumer.storageEventQueue.enabled` is `true` and that the pod does **not** carry `DISABLE_EVENT_CONSUMPTION=true`. The startup log names the resolved `EVENT_CONSUMER_SERVER` role.

(thumbnail-service-troubleshooting-queue-id-digest-check)=
### Queue id digest check

Both durable queue ids are derived from the sorted, deduplicated `NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES` by each binary at startup:

```text
thumbnail-service-storage-<sha256 hex>
thumbnail-service-worker-<sha256 hex>
```

The digest carries no queue-kind discriminator, so **the same prefix set yields the same digest on both ids**. In a Notification Service queue listing, or in each pod's startup log, compare the pair:

```text
thumbnail-service-storage-dc9ff58a…  ┐ same digest → both components agree on resourcePrefixes
thumbnail-service-worker-dc9ff58a…   ┘

thumbnail-service-storage-dc9ff58a…  ┐ different digest → one Deployment's configuration is stale
thumbnail-service-worker-053670e3…   ┘
```

Changing `notifications.eventConsumer.resourcePrefixes` rotates **both** queues to fresh ids on the next startup; work held on the superseded queues is not consumed. See {ref}`Durable queue self-provisioning <thumbnail-service-event-consumption-durable-queue-self-provisioning>`.

(thumbnail-service-troubleshooting-pods-crash-loop-at-startup)=
## Pods crash-loop at startup

Both binaries fail fast on misconfiguration and exit non-zero before binding a listener. The first `error` line in the pod log names the cause:

- **`required arguments were not provided`** — one of the six required `THUMBNAIL_*` processing defaults, `THUMBNAIL_PATH_TEMPLATE`, or (for the worker) an endpoint is unset. The message names the flag; see {ref}`Environment configuration reference <thumbnail-service-configuration-environment-configuration-reference>`.
- **`Missing required environment variable: NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES`** — the consumer is enabled with no prefixes. A blank entry in a comma-separated value (`a,,b`) also fails at parse time; Helm trims blank list entries before joining, so this is mostly seen with raw environment files.
- **`TLS endpoints are not supported`** — an endpoint was given with an `https://` scheme. Every outbound endpoint is plaintext HTTP/2 inside the cluster; TLS terminates at the ingress proxy. Supply a scheme-less or `http://` address.
- **`thumbnail path template must …`** — the template failed startup validation: an unknown variable, no source-identifying variable (`{resource_address_md5}` or both `{resource_path}` and `{resource_filename}`), or a render that is not an absolute address. See {ref}`Thumbnail destination path template <thumbnail-service-destination-path-template-thumbnail-destination-path-template>`.
- **Startup stalls for about a minute, then exits.** No usable service-identity bearer arrived within the 60-second readiness gate — a slow identity provider, wrong client credentials, or a token-provisioner sidecar that had not written its file yet. Both startup probes budget about 125 seconds for this wait; if liveness is killing the pod sooner, the startup probe has been disabled. See {ref}`Token readiness at startup <thumbnail-service-authentication-token-readiness-at-startup>`.
- **Both or neither identity mode enabled.** The chart refuses to render, and the binary refuses to start, unless exactly one of `serviceIdentity.clientCredentials.enabled` and `serviceIdentity.accessTokenFile.enabled` is `true`.
- **Worker fails to render with a memory-limit error.** `thumbnailWorker.resources.limits.memory` must cover `workerRuntime.maxConcurrentConversions × workerRuntime.maxTransformPeakBytes + 200Mi`. Raise the limit or lower the concurrency or budget together; see {ref}`Sizing <thumbnail-service-generation-pipeline-sizing>`.
- **The worker's `/healthz` port is already in use.** The worker binds `HEALTH_PORT` (Helm `workerRuntime.healthPort`) before anything else so probes can answer during the token wait; a port conflict is a startup failure.

(thumbnail-service-troubleshooting-health-returns-503)=
## `/health` returns 503

`GET /health` is the **readiness** probe and returns `503` with `{"status": "GRPC_UNAVAILABLE"}` whenever the gRPC side of the unified process is not serving — before the gRPC listener binds, or after it exits for any reason. `GET /health/live` is the dependency-free liveness probe and stays `200` while the HTTP server accepts requests. A pod that is live but not ready has usually failed to bind `GRPC_PORT` (Helm `service.grpcPort`, default `0.0.0.0:50051`); look for `failed to bind gRPC listener` in the log. The process signals the surviving protocol server to drain and exits, so the pod restarts rather than serving HTTP only.

(thumbnail-service-troubleshooting-consumer-cannot-subscribe)=
## The consumer cannot subscribe

- **`PERMISSION_DENIED` on `CreateDurableQueue`.** Logged as a warning; the service **attaches anyway**, assuming the queue was created out of band. That only works if it was. Grant `event-consumer-service:create-durable-queues` to the service identity, or create the queue with the notification helper scripts.
- **`PERMISSION_DENIED` on the consume stream.** Logged once at `error` naming the queue id and the reason, then retried with jittered exponential backoff. The identity lacks `event-consumer-service:consume-durable-queues`, or — for the worker, if the Notification deployment enforces fine-grained permissions — it has no `event_permissions` entry for `omni.thumbnail.generate`.
- **`UNAUTHENTICATED` on the stream.** The first occurrence refreshes the token and reconnects immediately; a second consecutive occurrence backs off (`Token was refreshed and rejected again`). Persistent rejection means the identity provider is issuing tokens the Notification Service does not accept.
- **`InvalidArgument` or `Unimplemented` on `CreateDurableQueue`.** Permanent; startup aborts. `Unimplemented` means the deployed Event Consumer service does not support client-specified queue ids.
- **The stream returns `NOT_FOUND`.** The queue was deleted underneath a live consumer. Both binaries re-provision it with their configured filters and reconnect without a restart (`re-provisioning before reconnect`); a failure to re-provision is logged at `error` with the queue id and backs off.

(thumbnail-service-troubleshooting-retryable-failures-accumulate)=
## Retryable failures accumulate

- **`thumbnail_worker_unacknowledged_deliveries` rises steadily.** Retryable failures are being negatively acknowledged and redelivered faster than they retire — a dependency (storage or the service's `AssociateThumbnail`) is persistently failing. `thumbnail_worker_failures_total{class}` says which stage; the paired `warn` line `Retryable failure; scheduling a spaced redelivery` names the destination.
- **`thumbnail_worker_nacks_total{reason="early_redelivery"}` climbs.** The broker is requeueing before the configured redelivery delay (`REDELIVERY_RECYCLE_INTERVAL`, Helm `workerRuntime.redeliveryRecycleInterval`, default `60s`) elapsed; the worker rejects the early copy without reprocessing. Past `NACK_EARLY_REDELIVERY_LIMIT` (default `5`) consecutive early arrivals for one destination, the worker recycles its stream (`recycling the stream (circuit-breaker role)`) to force spacing by not being connected.
- **`Deferral bound reached; negatively acknowledging immediately`.** More than `NACK_DEFERRAL_CONCURRENCY_LIMIT` (default `10`) deliveries were waiting out their delay at once; further failures are returned immediately and the anti-spin gate spaces them instead. Sustained, this means a dependency has been down long enough to saturate the bound.
- **A message keeps coming back and never dead-letters.** Only *permanent* failures are acknowledged; a retryable one is redelivered until the Notification Service's own bounded redelivery dead-letters it. Inspect and replay dead-lettered generate messages with the platform's `ManageDeadLetteredEvents` RPC. There is no per-item task store.
- **Failures climb while NACKs flatline.** Settlements are not reaching the broker — check `thumbnail_worker_stream_recycles_total` and the `NACK could not be delivered; recycling the stream (fallback role)` warning.

See {ref}`Redelivery and negative acknowledgement <thumbnail-service-event-consumption-redelivery-and-negative-acknowledgement>` and {ref}`Acknowledgement matrix <thumbnail-service-generation-pipeline-acknowledgement-matrix>`.

(thumbnail-service-troubleshooting-conversions-are-rejected-as-over-budget)=
## Conversions are rejected as over budget

A permanent `decode`-class failure whose message reports a computed peak and the configured budget means the resample would exceed `MAX_TRANSFORM_PEAK_BYTES` (default 1 GiB). The bound is set by the resample working buffer — `source_width × new_height × 16` bytes on top of the source and the result — so a source well inside the pixel bound can still peak in gigabytes for a small thumbnail (a 1,000,000 × 64 source is exactly 64 MP and costs about 1.77 GiB to reduce to 100 × 100). This is a deliberate rejection rather than an OOM kill. Raise `workerRuntime.maxTransformPeakBytes` **and** `thumbnailWorker.resources.limits.memory` together if such sources must be served. Sources over `MAX_SOURCE_BYTES` (Helm `workerRuntime.maxSourceBytes`, 64 MiB) or `MAX_DECODED_PIXELS` (Helm `workerRuntime.maxDecodedPixels`, 64,000,000) are refused the same way. See {ref}`Conversion bounds <thumbnail-service-generation-pipeline-conversion-bounds>`.

(thumbnail-service-troubleshooting-orphaned-thumbnail-blobs)=
## Orphaned thumbnail blobs

- **`thumbnail_worker_orphaned_blobs_total` increments.** A thumbnail was uploaded, then `AssociateThumbnail` rejected it **permanently** (`failure_kind: PERMANENT`). The worker acknowledges the delivery, deletes nothing — deleting could break a live association if the service committed but the response was lost — and logs `Uploaded thumbnail is orphaned` at `error` with the destination and identity. Reconcile from that address.
- **`Generated thumbnail blob deletion failed after its metadata was removed`.** During the storage-deleted sweep a service-generated blob could not be deleted; its metadata is already gone, so no enumeration will find it again. The log names the address.
- **Blobs remain after a source is deleted, without any error.** They are **caller-owned**: their association was `MANUAL`, or their address does not match the path template, so the sweep removed the metadata and left the blob (`leaving '<address>' in storage … treated as caller-owned`). This is by design; see {ref}`Blob deletion policy <thumbnail-service-associations-blob-deletion-policy>`.

(thumbnail-service-troubleshooting-401-versus-403)=
## `401` versus `403`

The service checks only that an `Authorization: Bearer <token>` header is **present**; the storage service does the authorizing. Read the two statuses as different problems:

- **`401 Unauthorized` / `UNAUTHENTICATED`** — no bearer header, a malformed one, or a token the storage service rejected (typically expired). A new token is needed; retrying the same one will fail again.
- **`403 Forbidden` / `PERMISSION_DENIED`** — the token was accepted and the caller is not authorized to read the resource. A new token will not help; fix the caller's permissions. Returned on the read path and on `CreateThumbnails`; a denied *write* during `AssociateThumbnail` is reported as `500` / an in-band permanent failure instead.
- **Every bearer-shaped token gets `200` from `GET /v1alpha/thumbnails`.** `DEVELOPMENT_MODE=true` is set. It returns fabricated data without calling storage and therefore performs no authorization at all. Remove it — it is never a chart value and only reaches a pod through `extraVars`. See {ref}`Inbound request authentication <thumbnail-service-authentication-inbound-request-authentication>`.

(thumbnail-service-troubleshooting-get-returns-fewer-thumbnails-than-expected)=
## `GET /v1alpha/thumbnails` returns fewer thumbnails than expected

- **Exactly 1,000 entries.** The result is ordered by thumbnail address and **truncated** at 1,000 with no continuation token; the remainder is discarded, not paginated. Narrow the query with `width`, `height`, `encoding_format`, `transform_type`, or `crop_strategy`.
- **Some entries missing, others present.** Stored entries the service cannot read — no thumbnail address, or an unparseable key — are skipped rather than failing the call. Each skip is logged at `warn` by metadata key, followed by `Returned a partial thumbnail list` with the skipped keys.
- **Empty list for a target that has thumbnails.** Associations are keyed on the target's `resource_address`, so the query address must match the canonical form the writer used (percent-encoding differences matter). An empty list is a successful read of nothing, never `404`.
- **`502` on both read and disassociate for one address.** The target carries more `thumbnail_*` metadata than storage returns in a single response — the failure mode the conformance suite's over-bound case can cause on a non-conformant deployment. See {ref}`Querying thumbnails <thumbnail-service-associations-querying-thumbnails>`.

(thumbnail-service-troubleshooting-unexpected-thumbnail-output)=
## Unexpected thumbnail output

- **WebP thumbnails are much larger than JPEG ones.** WebP is encoded **lossless** and `quality` is ignored for it — the WebP encoder has no lossy mode. Use JPEG where size matters.
- **Padding around thumbnails.** The chart default transform is `FIT` with `preserveAspectRatio: true`: the whole source is scaled into the box and the remainder padded — transparent for PNG, opaque white for JPEG. Choose `CROP` or `SCALE_AND_CROP` to fill the box by discarding edges instead.
- **`SMART` crop has no visible effect.** A crop strategy only applies to `CROP` and `SCALE_AND_CROP`. Under `FIT` (the default) the crop analyzer never runs; the worker logs this at `debug`.
- **Wrong destination layout.** Substitution is verbatim: `{resource_path}/.thumbs/…` colocates under the source, `{resource_path}.thumbs/…` creates a sibling directory whose name ends in `.thumbs`. See {ref}`Thumbnail destination path template <thumbnail-service-destination-path-template-thumbnail-destination-path-template>`.

(thumbnail-service-troubleshooting-useful-commands)=
## Useful commands

```bash
# pods, and which Deployment is unhealthy
kubectl get pods -n storage-apis -l app.kubernetes.io/instance=thumbnail-service

# service log at debug for one upload (set observability.logging.level first)
kubectl logs -n storage-apis deploy/thumbnail-service -c thumbnail-service --since=5m | grep -i "skipping create event"

# worker metrics without a collector
kubectl port-forward -n storage-apis deploy/thumbnail-service-thumbnail-task-worker 8080:8080
curl -s http://localhost:8080/metrics | grep thumbnail_worker_

# readiness of each surface
curl -s http://thumbnail-service.storage-apis.svc.cluster.local:3000/health
curl -s http://thumbnail-service.storage-apis.svc.cluster.local:3000/health/live

# both derived queue ids, from the startup logs
kubectl logs -n storage-apis deploy/thumbnail-service -c thumbnail-service | grep -o "thumbnail-service-storage-[0-9a-f]*" | head -1
kubectl logs -n storage-apis deploy/thumbnail-service-thumbnail-task-worker -c thumbnail-worker | grep -o "thumbnail-service-worker-[0-9a-f]*" | head -1

# roll back the service first, then drain the worker queue before scaling workers down
helm history thumbnail-service -n storage-apis
helm rollback thumbnail-service <REVISION> -n storage-apis
```

The `scripts-notifications` helpers shipped with the service source (`consume-durable-queue.sh`, `drain-queue.sh`, `update-durable-queue.sh`, `delete-durable-queue.sh`) drive the Notification Service directly with `grpcurl` for inspection, draining, and deletion of durable queues; self-provisioning makes `create-durable-queue.sh` optional.

(thumbnail-service-troubleshooting-see-also)=
## See also

- {ref}`Observability <thumbnail-service-observability-observability>` — every metric and log line referenced above, with their attributes.
- {ref}`Storage event consumption <thumbnail-service-event-consumption-storage-event-consumption>` — provisioning outcomes, eligibility rules, and the redelivery mechanism.
- {ref}`Thumbnail generation worker <thumbnail-service-generation-pipeline-thumbnail-generation-worker>` — the acknowledgement matrix, failure classes, and conversion bounds.
- {ref}`Association metadata and queries <thumbnail-service-associations-association-metadata-and-queries>` — the read path's cap and skip rules, and the blob deletion policy.
- {ref}`Thumbnail Service Configuration <thumbnail-service-deployment-thumbnail-service-configuration>` — rollout ordering and the rollback procedure.
