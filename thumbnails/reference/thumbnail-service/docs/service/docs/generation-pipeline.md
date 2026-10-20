(thumbnail-service-generation-pipeline-thumbnail-generation-worker)=
# Thumbnail generation worker

The **thumbnail task-worker** (`task-worker`) is the conversion half of the pipeline: a separate binary and a separate Deployment rendered by the same Helm chart, scaled independently of the API. It consumes `omni.thumbnail.generate` work from its own durable Notification queue — **WorkerEventQueue** — downloads the source from storage, converts it, uploads the thumbnail to the destination the service resolved, and asks Thumbnail Service to record the association over gRPC. It serves no API.

This page describes the worker's boundaries, its queue, the consume loop, the acknowledgement rules, the classification of failures, the conversion pipeline and its bounds, the supported transforms and encodings, the association call, its probe surface, and how to size it. The producing side — how the service decides what to generate — is in {ref}`Storage event consumption <thumbnail-service-event-consumption-storage-event-consumption>`.

(thumbnail-service-generation-pipeline-role-and-boundaries)=
## Role and boundaries

The worker does:

- Consume `omni.thumbnail.generate` from WorkerEventQueue, with explicit acknowledgements.
- Download the source with the storage `FileObjectService.Read` RPC.
- Convert the source with its in-process image library, using the specification carried on the message.
- Upload the result with `FileObjectService.Write`, to the exact `destination_resource_address` on the message.
- Call `AssociateThumbnail` on Thumbnail Service.
- Report outcomes through metrics, structured logs, and the platform dead-letter queue.

The worker does not:

- Consume `omni.storage.created` or any other storage event.
- Serve REST or gRPC. It opens one listener, for probes and metrics only.
- Read any `THUMBNAIL_*` processing default. Every field must already be on the message — a missing field
  fails validation, it is never filled in locally.
- Derive, alter, or fall back on a destination address.
- Write association metadata itself, or publish `omni.thumbnail.associated`.
- Keep a task store. Per-item status is not queryable.

The worker reaches the event broker only through the published Notification API at its configured endpoint. A customer may run a conformant implementation of that API in place of the Notification Service, provided it redelivers unacknowledged messages on reconnect and applies bounded redelivery leading to dead-lettering; a service that drops unacknowledged messages on disconnect loses work.

(thumbnail-service-generation-pipeline-workereventqueue)=
## WorkerEventQueue

On startup — after the probe surface is listening and the service identity is ready — the worker ensures its queue exists with `CreateDurableQueue`, exactly as the orchestrator does for StorageEventQueue (see {ref}`Durable queue self-provisioning <thumbnail-service-event-consumption-durable-queue-self-provisioning>`). Its id is derived, not configured:

```text
thumbnail-service-worker-<sha256 hex of the sorted, de-duplicated, comma-joined resource prefixes>
```

The digest is the same one StorageEventQueue's id ends in for the same `NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES`; the `-worker-` segment alone keeps the two ids distinct. The queue carries one filter group for `omni.thumbnail.generate` with one greedy-prefix filter per configured prefix, matched against the **source** address the envelope names. An empty filter list binds a queue that receives nothing while every call succeeds, which is why the prefixes are required: a worker whose prefix set has drifted from the orchestrator's sits at zero deliveries with no error anywhere, indistinguishable from no eligible files arriving. The digest comparison in a queue listing is the check for that.

A queue deleted underneath a running worker surfaces as `NOT_FOUND` on the stream; the worker re-provisions and reconnects without a restart.

(thumbnail-service-generation-pipeline-consume-loop-and-concurrency)=
## Consume loop and concurrency

The worker opens `ConsumeDurableEventsWithAcks` on its queue and processes at most `MAX_CONCURRENT_CONVERSIONS` deliveries at once.

Reconnects follow the same causes and jittered backoff as the orchestrator's consumer (stream end, transport error, `UNAUTHENTICATED` after refresh, token near expiry, `NOT_FOUND`, and the two recycle triggers); a quiet healthy stream is never rebuilt for silence. Every 1 s the loop checks whether the service token is near expiry and, if so, refreshes it and reopens the stream.

On SIGTERM the worker stops reading, marks itself draining (both probes fail), and gives in-flight conversions `SHUTDOWN_DRAIN_TIMEOUT` (default `30s`) to finish. Conversions that complete send their own acknowledgement; anything still running is aborted and its delivery stays unacknowledged for redelivery. Deferred negative acknowledgements are flushed, and the stream is held open for a 2 s grace so queued acknowledgements reach the wire. `thumbnailWorker.terminationGracePeriodSeconds` (default `45`) must exceed the drain timeout.

(thumbnail-service-generation-pipeline-acknowledgement-matrix)=
## Acknowledgement matrix

The worker tries each delivery exactly once. If it retried right away, one slow dependency could tie up a
conversion slot and block other work. Instead, a retryable failure gets a negative acknowledgement, and the
Notification Service redelivers the message later, once the dependency has had time to recover.

| Outcome | Acknowledgement | Where it shows up |
|---|---|---|
| Converted, uploaded, and associated | **ACK** | `omni.thumbnail.associated` published by the service; `thumbnail_worker_outcomes_total{outcome="success"}`; `info` log `Thumbnail generated and associated` |
| **Permanent** failure (validation, unsupported format, source missing, a bound exceeded, permanent association rejection) | **ACK**, after an `error` log `Permanently rejecting a generate message` naming the destination and failure class | `thumbnail_worker_outcomes_total{outcome="permanent_failure"}`, `thumbnail_worker_failures_total{class=…}` |
| **Retryable** failure (storage or Thumbnail Service unavailable, deadline exceeded, transient association rejection) | **Negative acknowledgement**, sent after `REDELIVERY_RECYCLE_INTERVAL` (default `60s`, floor `5s`) | `warn` logs `Retryable failure; withholding the acknowledgement` and `Retryable failure; scheduling a spaced redelivery`; `thumbnail_worker_nacks_total{reason="retryable_failure"}`; redelivery, and eventually the platform dead-letter queue |
| Redelivery arriving before its key's next-attempt time | Negative acknowledgement without reprocessing | `thumbnail_worker_nacks_total{reason="early_redelivery"}`; after `NACK_EARLY_REDELIVERY_LIMIT` (default `5`) consecutive early arrivals the stream is recycled |

Recycled means the worker closes its stream and opens a new one right away. The Notification Service then
redelivers everything left unacknowledged.

These are the worker's metrics. `thumbnail_worker_outcomes_total` counts messages by how they ended — the
`outcome` label matches the rows in this table (`success`, `permanent_failure`, `retryable_failure`).
`thumbnail_worker_failures_total` counts failures by which pipeline stage produced them, in the `class`
label. Both are OpenTelemetry counters, exported over OTLP and also returned as simple key-value lines with labels,
types, and help text on the worker's `/metrics` endpoint. See {ref}`Metrics <thumbnail-service-observability-worker-metrics>` for the full metric
list, and {ref}`Failure classes <thumbnail-service-generation-pipeline-failure-classes>` below for what each
`class` value means.

Permanent failures are acknowledged deliberately: a poison message that kept its delivery would loop forever and never reach the dead-letter queue, because the delivery counter advances only on an actual redelivery. The error log before the acknowledgement is the only durable trace such a message leaves.

The **delivery key** that spaces successive attempts is the message's `destination_resource_address` — a deterministic function of source identity and specification that survives redelivery where the delivery tag does not. Held negative acknowledgements are bounded by `NACK_DEFERRAL_CONCURRENCY_LIMIT` (default `10`); past the bound a failure is negatively acknowledged immediately. The worker is idempotent on the destination: a duplicate delivery overwrites the same blob (the write carries no optimistic lock) and the repeated `AssociateThumbnail` converges on the same metadata key.

(thumbnail-service-generation-pipeline-failure-classes)=
## Failure classes

Every failed outcome carries exactly one class, which becomes the `class` label of `thumbnail_worker_failures_total`. Retryability of RPC failures follows the gRPC status: `UNAVAILABLE`, `DEADLINE_EXCEEDED`, `RESOURCE_EXHAUSTED`, `ABORTED`, `INTERNAL`, `UNKNOWN`, `CANCELLED`, and `DATA_LOSS` are retryable; `INVALID_ARGUMENT`, `NOT_FOUND`, `ALREADY_EXISTS`, `PERMISSION_DENIED`, `UNAUTHENTICATED` (after the one refresh-and-reissue every call performs), `FAILED_PRECONDITION`, `OUT_OF_RANGE`, and `UNIMPLEMENTED` are permanent.

| Class | Raised when | Retryable? |
|---|---|---|
| `validation` | The message fails the generate contract (missing field, unsupported `schema_version`, dimension outside `1..=8192`, JPEG/WebP quality outside `1..=100`) or worker validation (blank identity, blank or non-absolute destination, destination equal to the source). | Permanent — except a **newer** `schema_version`, which withholds the acknowledgement so a rolling upgrade never dead-letters work an updated worker could handle. |
| `unsupported_processor` | A processor other than image-to-image. Declared; not produced today because the service publishes only image-to-image specifications. | Permanent |
| `credential` | No outbound bearer token is available, or the auth headers could not be built. | Retryable when the token source may recover; permanent for malformed credentials. |
| `source_missing` | Storage `Read` returned `NOT_FOUND` — typically a source deleted while its CREATE was in flight. | Permanent |
| `download` | Any other `Read` failure, or storage answered with a redirect the worker does not follow. | Per gRPC status; redirect permanent |
| `unsupported_format` | No decoder recognizes the bytes. | Permanent |
| `decode` | The bytes do not decode, the source is empty, or a bound was exceeded (source bytes, decoded pixels, transform peak). | Permanent — except a decode **panic**, which is treated as a worker defect and retried. |
| `encode` | The requested output format could not be written. | Permanent |
| `upload` | Storage `Write` failed, returned no resource identity, or redirected. | Per gRPC status; a missing identity is retryable |
| `associate` | `AssociateThumbnail` failed on the transport, or returned `success: false`. | Transport failures per gRPC status; in-band failures per the response's `failure_kind` — `PERMANENT` acknowledges, `RETRYABLE`, `UNSPECIFIED`, and absent are retried. |

(thumbnail-service-generation-pipeline-pipeline-stages)=
## Pipeline stages

Each delivery is processed in one span, `thumbnail.generate`, that continues the publisher's trace when the message carries W3C context (see {ref}`Tracing <thumbnail-service-observability-tracing>`).

1. **Parse and validate the contract.** The message must carry `schema_version` `1`, a source `resource_address` and (for `FILE` targets) `resource_identity`, a `destination_resource_address`, a `target_type`, and a specification with every field present: `width`, `height`, `encoding_format`, `transform_type`, `crop_strategy`, `quality`, `preserve_aspect_ratio`. Dimensions must be in `1..=8192`; quality in `1..=100` only for `JPEG` and `WEBP`.
2. **Validate the destination.** It must be non-blank and absolute, and must not equal the source after canonicalization — the last line of defense against a thumbnail overwriting its own source.
3. **Obtain the service credential.** The worker authenticates to storage and to Thumbnail Service as the service identity, never with a caller token (see {ref}`Which token is used for which call <thumbnail-service-authentication-which-token-is-used-for-which-call>`).
4. **Download** (`thumbnail.download`). `FileObjectService.Read` by the source `resource_identity`, bounded by `MAX_SOURCE_BYTES` on both the declared object size and the running total, and by the per-call deadline `OUTBOUND_REQUEST_TIMEOUT` (default `30s`), so a hung dependency releases its conversion slot.
5. **Convert** (`thumbnail.convert`). Sniff the format from the bytes, check every bound before allocating, flatten alpha onto white if the output is JPEG, apply the transform, encode. Source bytes are released after decoding.
6. **Upload** (`thumbnail.upload`). `FileObjectService.Write` to the destination, verbatim, with no optimistic lock — last writer wins, which is what makes duplicates converge. The write returns the storage-assigned `resource_identity` of the thumbnail; a write that completes without one is a retryable upload failure.
7. **Associate** (`thumbnail.associate`). Call `AssociateThumbnail` on Thumbnail Service; see {ref}`Association call <thumbnail-service-generation-pipeline-association-call>`.
8. **Acknowledge** per the {ref}`matrix above <thumbnail-service-generation-pipeline-acknowledgement-matrix>` and log `Thumbnail generated and associated` with destination, source, and elapsed time.

(thumbnail-service-generation-pipeline-conversion-bounds)=
## Conversion bounds

Three bounds are checked **before** allocation, so an oversized input is a deterministic permanent failure with a clear reason rather than an out-of-memory kill that would leave the delivery unacknowledged and reproduce on redelivery.

| Bound | Variable (Helm key) | Default | What it limits |
|---|---|---|---|
| Source bytes | `MAX_SOURCE_BYTES` (`workerRuntime.maxSourceBytes`) | `67108864` (64 MiB) | The encoded source, on the declared size and on the running download total. |
| Decoded pixels | `MAX_DECODED_PIXELS` (`workerRuntime.maxDecodedPixels`) | `64000000` (64 MP, decimal — roughly 8000×8000) | The decoded image, from the header before allocation, plus a decoder-level allocation cap of 4 bytes per pixel as a second defense against a lying header. |
| Transform peak bytes | `MAX_TRANSFORM_PEAK_BYTES` (`workerRuntime.maxTransformPeakBytes`) | `1073741824` (1 GiB) | The peak resident memory of one conversion. **This, not the pixel bound, is what the pod's memory limit is sized against.** |

The peak is set by the **resample**, not the decode. A resize runs in two passes, and the first allocates a working buffer of `source_width × new_height` at 16 bytes per pixel while both the source and the result are resident:

```text
peak = source_width × source_height × source_bytes_per_pixel
     + source_width × new_height × 16
     + new_width × new_height × source_bytes_per_pixel
     + extra resident buffers (FIT canvas, SMART crop copy)
```

The working buffer's width comes from the **source**, so it does not shrink with the request: a 1,000,000×64 image is exactly 64 MP and passes the pixel bound, yet a 100×100 thumbnail of it would peak near 1.7 GiB and is refused. The largest conversion the API contract permits — a 64 MP source resampled to 8192×8192 — peaks near 2 GiB and is refused at the default budget by design; the realistic worst case for the chart's `256x256,512x512` sizes is a 64 MP 16-bit gray-plus-alpha PNG flattened for JPEG output, about 553 MiB, leaving roughly 1.85× headroom. Raise `MAX_TRANSFORM_PEAK_BYTES` and the memory limit together if a deployment must serve the ceiling. Derived dimensions are computed without saturation; an overflow counts as exceeding the budget. A rejection reports the computed peak and the configured budget.

(thumbnail-service-generation-pipeline-transforms-and-crop-strategies)=
## Transforms and crop strategies

Geometry reproduces the documented Pillow behavior for every contract value, including truncation and rounding, with Lanczos resampling. Anything unimplemented is a permanent validation failure, never a silent substitution.

| `transform_type` | Behavior | Uses `crop_strategy`? | `preserve_aspect_ratio` |
|---|---|---|---|
| `SQUARE` | Center-crop to the largest square, then resize to `min(width, height)` squared. The result is always square even when the requested box is not. | No (always center) | Ignored |
| `SCALE_BY_WIDTH` | Width becomes `width`; height scales proportionally (at least 1). | No | Ignored |
| `SCALE_BY_HEIGHT` | Height becomes `height`; width scales proportionally. | No | Ignored |
| `CROP` | Crop a `width × height` box at the anchor. A box larger than the source yields the source. With `true`, the source is first scaled to **cover** the box. | Yes | Pre-scale to cover when `true` |
| `FIT` | With `true`: scale the whole source to fit inside the box, then center it on a `width × height` canvas — **opaque white** for JPEG, **transparent** for alpha-capable outputs. With `false`: an exact resize. | No | Letterbox vs exact resize |
| `SCALE_AND_CROP` | Scale to cover, then crop the overflow at the anchor. | Yes | Ignored (always covers) |
| `THUMBNAIL` | Fit inside the box and **never upscale**; an image that already fits is returned unchanged. | No | Ignored |
| `RESIZE` | Exact resize to `width × height`; aspect is ignored. | No | Ignored |

The chart's default transform is `FIT` with `preserveAspectRatio: "true"`: nothing is cut off, every thumbnail is exactly the requested size, and a source whose aspect ratio is far from the box shows visible padding.

`crop_strategy` anchors the crop for `CROP` and `SCALE_AND_CROP`: `CENTER`, `TOP`, `BOTTOM`, `LEFT`, `RIGHT`, `TOP_LEFT`, `TOP_RIGHT`, `BOTTOM_LEFT`, `BOTTOM_RIGHT`, or `SMART`. `SMART` is content-aware: the worker analyzes the image, crops the region the analyzer returns, and resizes it to the target. It **falls back to a center crop** when a target dimension is zero, the analyzer errors, or the returned region does not lie inside the source, so a request can never turn into a panic and a redelivery loop. For every other transform the strategy is inert; with the chart default `FIT` the analyzer never runs, and the worker logs at `debug` that the smart crop does not apply.

(thumbnail-service-generation-pipeline-output-encodings-and-quality)=
## Output encodings and quality

Decodable inputs are `bmp`, `gif`, `ico`, `jpeg`/`jpg`, `png`, `tif`/`tiff`, and `webp`, sniffed from the bytes rather than the extension; animated GIF and WebP contribute their first frame only. `svg`, `heic`/`heif`, and `avif` are not decodable and are excluded upstream by the orchestrator's eligibility check.

| Output | Alpha | `quality` |
|---|---|---|
| `JPEG` | Flattened onto white before the transform | **Honored**, `1..=100` |
| `PNG` | Kept | Ignored |
| `GIF` | Kept, single frame | Ignored |
| `WEBP` | Kept | **Ignored — WebP is encoded lossless.** The WebP encoder has no lossy mode, so a WebP-configured deployment produces noticeably larger thumbnails than a lossy encoder would. |

The API contract validates `quality` in `1..=100` for both `JPEG` and `WEBP` (so the value is well-formed on either transport), while the encoder applies it to JPEG alone.

(thumbnail-service-generation-pipeline-association-call)=
## Association call

After a successful upload the worker calls `AssociateThumbnail` on `THUMBNAIL_SERVICE_GRPC_ENDPOINT` — in the chart, this release's own Service at `<service name>:<service.grpcPort>`; it is not a values key. The request carries:

| Field | Value |
|---|---|
| `thumbnail.resource_address` | The `destination_resource_address` from the message |
| `thumbnail.resource_identity` | The identity storage returned from the `Write` — never a worker-constructed value |
| `association_target` | The message's source `resource_address` and `resource_identity` |
| `target_type` | The message's `FILE` or `FOLDER` |
| `specification` | The resolved image-to-image specification, every field round-tripped unchanged, including an explicit `preserve_aspect_ratio: false` |

The call carries a per-call deadline of `OUTBOUND_REQUEST_TIMEOUT`, injects the worker's trace context so the service-side span is a child of the conversion, and on `UNAUTHENTICATED` refreshes the service token once and reissues the identical request once — convert and upload are not repeated. Thumbnail Service answers `success: true`, or `success: false` with an `error_message` and an enumerated `failure_kind` that tells the worker whether to acknowledge (`PERMANENT`) or withhold (`RETRYABLE`; an absent or unrecognized value is treated as retryable so an older service never makes a newer worker acknowledge work it should retry). The service publishes `omni.thumbnail.associated` after its own write; the worker publishes nothing.

A **permanent** association rejection after a successful upload leaves the blob at its destination with no metadata pointing at it. The worker acknowledges the delivery, increments `thumbnail_worker_orphaned_blobs_total`, and logs at `error` (`Uploaded thumbnail is orphaned: associate failed permanently …`) with the destination and identity so an operator can reconcile — it never deletes, because a permanent transport error cannot distinguish "the service rejected this" from "the service committed it and the response was lost", and deleting in the second case would break a live association. A retryable rejection withholds the acknowledgement; the redelivery overwrites the same destination and associates it.

(thumbnail-service-generation-pipeline-probes)=
## Probes

The worker opens one listener on `HEALTH_PORT` (default `8080`; Helm `workerRuntime.healthPort`) **before** it waits for the service identity or provisions its queue, so a cold start never fails a liveness probe while the token gate is still open. The distroless image has no shell, so the binary can probe itself with `--probe-ready` for container health checks; Kubernetes uses the HTTP paths directly.

| Path | `200` when | `503` when | Body |
|---|---|---|---|
| `/healthz` (startup and liveness) | Not draining | Draining | `ok` / `draining` |
| `/readyz` (readiness) | The consume stream is established **and** the worker is not draining | Otherwise | `ready` / `not ready` |
| `/metrics` | Always | — | OpenTelemetry metrics, returned as simple key-value lines with labels, types, and help text (see {ref}`Worker metrics <thumbnail-service-observability-worker-metrics>`) |

These are OpenTelemetry metrics. They are exported over OTLP and also returned as simple key-value lines with
labels, types, and help text on this endpoint.

Liveness is deliberately weaker than readiness: a worker reconnecting after a stream error is taken out of service, not restarted — a restart abandons its in-flight work and fixes nothing. Readiness follows the stream, so a replica that has no established stream is not Ready; a worker that stays Ready while consuming nothing is the failure the probe exists to catch.

(thumbnail-service-generation-pipeline-sizing)=
## Sizing

`thumbnailWorker.replicaCount` (and the worker HPA, on by default at 2–20 replicas targeting 70% CPU) is the "number of workers" knob: the worker is single-process and scales horizontally, one blast radius per pod. `workerRuntime.maxConcurrentConversions` controls how much each replica does at once, and the two are sized together.

**Memory** is bounded by the transform peak budget, one budget per in-flight conversion. The chart refuses to render when the limit cannot cover it:

```text
limits.memory  ≥  maxConcurrentConversions × maxTransformPeakBytes + 200 MiB idle
                  2 × 1 GiB + 200 MiB ≈ 2.2 GiB  →  2304Mi
```

The default request of `750Mi` is the typical JPEG working set measured under load, not the worst case; the gap is intentional so the autoscaler packs on observed draw. Anything over the budget is a permanent conversion failure, not an OOM kill.

**CPU** is sized against concurrency from measurement: each slot averages about 0.6 of a core across download, convert, upload, and associate, so two slots draw roughly 1.2–1.4 cores — the `1300m` request keeps utilization above the HPA target under backlog — and the `2500m` limit is the ceiling two single-threaded conversions can usefully draw plus headroom for acknowledgements and probes. The CPU **limit**, not the node, sizes the worker's internal concurrency (the cgroup quota is read and rounded down), which is safe only because conversion runs off the main processing loop. Throughput for reference: about 600 conversions per minute per worker on the load-test corpus.

See {ref}`Scaling <thumbnail-service-deployment-scaling>` for the chart values and {ref}`Worker environment variables <thumbnail-service-configuration-worker-environment-variables>` for every variable named here.

(thumbnail-service-generation-pipeline-references)=
## References

- [Pillow — Image.resize](https://pillow.readthedocs.io/en/stable/reference/Image.html#PIL.Image.Image.resize) — the resize geometry and filter behavior this worker reproduces, including Lanczos resampling

(thumbnail-service-generation-pipeline-see-also)=
## See also

- {ref}`Storage event consumption <thumbnail-service-event-consumption-storage-event-consumption>` — the producer of the generate events this worker consumes, and the first acknowledgement domain.
- {ref}`Thumbnail destination path template <thumbnail-service-destination-path-template-thumbnail-destination-path-template>` — how the destination the worker writes to verbatim is resolved.
- {ref}`Association metadata and queries <thumbnail-service-associations-association-metadata-and-queries>` — what `AssociateThumbnail` records and how duplicates converge on one key.
- {ref}`Service notifications <thumbnail-service-notifications-service-notifications>` — the `omni.thumbnail.generate` wire contract, including `schema_version` and trace context.
- {ref}`Observability <thumbnail-service-observability-observability>` — every worker metric family, the `thumbnail.generate` span, and log conventions.
- {ref}`Authenticating with an OpenID Connect Identity Provider <thumbnail-service-authentication-authenticating-with-an-openid-connect-identity-provider>` — the service identity the worker uses for storage and association calls.
- {ref}`Troubleshooting <thumbnail-service-troubleshooting-troubleshooting>` — accumulating retryables, orphaned blobs, and over-budget conversions.
