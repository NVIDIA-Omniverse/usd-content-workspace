(thumbnail-service-event-consumption-storage-event-consumption)=
# Storage event consumption

Thumbnail Service generates thumbnails automatically when files land in storage. It holds a durable subscription — **StorageEventQueue** — to the Notification Service and consumes two storage lifecycle events: `omni.storage.created`, which drives thumbnail generation, and `omni.storage.deleted`, which drives cleanup of every association the deleted source carried. Each delivery decides its own acknowledgement, and the two event types never share an outcome.

This page covers when the consumer runs, how it provisions and identifies its queue, which CREATE events are eligible, what an acknowledgement means, how retryable failures are returned to the queue, how the consume stream is kept alive, and what the DELETE handler does. The work it publishes — one `omni.thumbnail.generate` event per thumbnail — is consumed by the worker on a second, separate queue; see {ref}`Thumbnail generation worker <thumbnail-service-generation-pipeline-thumbnail-generation-worker>`.

(thumbnail-service-event-consumption-when-the-consumer-runs)=
## When the consumer runs

Thumbnail Service runs the consumer in the same process that serves REST and gRPC — there is no separate consumer container. Two settings gate it:

| Setting | Values | Effect |
|---|---|---|
| `DISABLE_EVENT_CONSUMPTION` | `true` / `false` (default `false`) | Hard off switch, checked first. `true` disables consumption regardless of the role below. |
| `EVENT_CONSUMER_SERVER` | `HTTP` (default), `GRPC`, `NONE` | Which role this process assumes. The single process adopts whatever role it parses, so `HTTP` and `GRPC` both enable consumption; `NONE` disables it. |

In the Helm chart this is `notifications.eventConsumer.storageEventQueue.enabled` (default `true`): when enabled the orchestrator receives `NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT` and `NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES`; when disabled it receives `DISABLE_EVENT_CONSUMPTION=true`. Once consumption is enabled and assigned, both of those variables are **required** — a missing value is a startup failure naming the variable (see {ref}`Server environment variables <thumbnail-service-configuration-server-environment-variables>`).

The consumer runs under a supervisor with a continuous restart policy: a clean return restarts after 5 s, an initialization failure after 10 s, and a panic after 10 s. The consumer does not open its stream until a usable service-identity token is available (see {ref}`Token readiness at startup <thumbnail-service-authentication-token-readiness-at-startup>`).

(thumbnail-service-event-consumption-durable-queue-self-provisioning)=
## Durable queue self-provisioning

The assigned consumer always ensures its durable queue exists on startup by calling `CreateDurableQueue` (create-if-absent). There is no opt-out and no pre-creation step; the manual `scripts-notifications/update-durable-queue.sh`, `drain-queue.sh`, and `delete-durable-queue.sh` scripts remain useful only for update, drain, and delete workflows.

**Queue id.** StorageEventQueue's id is not configured directly. It is derived at startup as

```text
thumbnail-service-storage-<sha256 hex of the sorted, de-duplicated, comma-joined resource prefixes>
```

`thumbnail-service` is a hardcoded literal, so every deployment's queues are recognizable in a Notification queue listing. The worker derives **WorkerEventQueue**'s id the same way with a `-worker-` segment, and the digest carries no queue-kind discriminator — so for one prefix set both ids end in the **same** digest and differ only in that segment. That is the intended operator check: matching digests across the `-storage-` and `-worker-` pair mean both components agree on `NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES`; differing digests mean one side's configuration is stale. Changing the prefix set rotates both queues to fresh ids on the next startup; reordering or duplicating entries does not.

**Filters.** `NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES` (Helm `notifications.eventConsumer.resourcePrefixes`, a YAML list joined with `,`) is a list of storage-container URL prefixes. The consumer builds two filter groups — one for `omni.storage.created`, one for `omni.storage.deleted` — each carrying one greedy-prefix filter per configured prefix, so one deployment can subscribe across several containers. Each filter matches the storage object's address against a configured prefix and is *greedy*: once the address starts with the prefix, it matches no matter how deep the object sits below it — a filter on a container root matches every object in every subfolder, not just the ones directly in it. A group's filters are OR'd, so an event matches if its address starts with any configured prefix. Use each container root, not a subdirectory and not the thumbnail template. Blank delimited entries (`a,,b`) are rejected at parse time; the Helm list path trims and drops blank entries before joining. An empty prefix list is refused rather than treated as a catch-all.

**Provisioning outcomes.** Each `CreateDurableQueue` attempt is bounded by `STREAM_TIMEOUT_SECS` (default `90`). The response must echo the requested `queue_id`; a different id fails provisioning and aborts startup rather than attaching to a queue the configuration does not name.

| gRPC status from the Notification Service | Behavior |
|---|---|
| `OK` (created or already existed) | `[PROVISION] Durable queue created` or `[PROVISION] Durable queue already existed`; the consumer attaches. |
| `PERMISSION_DENIED` | **Orchestrator only:** logged as a warning, and the consumer **attaches anyway**, assuming an out-of-band queue exists (backward-compatible with consume-only identities). This only works if the queue was already created. The worker has no such fallback — it treats `PERMISSION_DENIED` like any other provisioning error and aborts startup, so a consume-only identity must still be granted `create-durable-queues` for WorkerEventQueue. |
| `UNAUTHENTICATED` | The service-identity token is refreshed once and the call reissued once; a second rejection fails startup. |
| `INVALID_ARGUMENT`, `UNIMPLEMENTED` | Permanent misconfiguration; startup fails. |
| `UNAVAILABLE`, `DEADLINE_EXCEEDED`, connection errors | Retried up to `NOTIFICATIONS_PROVISIONING_MAX_RETRIES` times (default `4`; `0` disables retries) with the same jittered exponential backoff the reconnect path uses, then startup fails. A shutdown signal during backoff aborts cleanly. |

The service identity needs `event-consumer-service:create-durable-queues` and `event-consumer-service:consume-durable-queues`; see {ref}`Permissions the service identity needs <thumbnail-service-authentication-permissions-the-service-identity-needs>`.

(thumbnail-service-event-consumption-eligibility)=
## Eligibility

Every `omni.storage.created` event passes through the following checks in order; the first hit decides. Skips and rejections are **acknowledged** — redelivery cannot change the outcome — and skips are logged at `debug` level as `Skipping CREATE event` with the address and reason.

Checks 4–6 are three recursion-prevention signals, in order of preference, that stop the service from
re-processing its own generated output as a new source file. Storage does not yet populate either event-based
signal, so checks 4 and 5 are both inert today; check 6 — the destination path-template shape — is the guard
actually doing the work. See {ref}`Generated-output recursion prevention <thumbnail-service-destination-path-template-generated-output-recursion-prevention>`
for the mechanism that's currently active.

| # | Check | Outcome | Notes |
|---|---|---|---|
| 1 | `resource_address` is blank | Reject, ACK | Permanent: a malformed event cannot be dispatched. |
| 2 | `resource_identity` is blank or absent | Reject, ACK | Permanent; checked before the folder check. |
| 3 | Address has a folder shape: ends with `/`, ends with `.folder`, or contains `/.folder/` | Skip, ACK | Files that do not require a generated thumbnail can be skipped. |
| 4 | Event carries the `generated_by_thumbnail_pipeline` marker set to `true` | Skip, ACK | The event-level marker saying this write came from the thumbnail pipeline. Storage does not set it yet, so this never matches today. |
| 5 | `principal_identity` equals `GENERATED_THUMBNAIL_PRINCIPAL_IDENTITY` (when configured) | Skip, ACK | Compares the event's writer principal to a configured value naming the worker's own identity. No longer a Helm value; inject via `thumbnailService.extraVars` if an environment needs it. Storage does not propagate a writer principal yet, so this never matches today. |
| 6 | Address matches the rendered shape of `THUMBNAIL_PATH_TEMPLATE` | Skip, ACK | The active recursion guard; see {ref}`Generated-output recursion prevention <thumbnail-service-destination-path-template-generated-output-recursion-prevention>`. |
| 7 | Address has no file extension | Skip, ACK | |
| 8 | Extension is not in the supported set | Skip, ACK | |
| — | Otherwise | **Eligible** | Dispatch proceeds. |

The folder-shape check (row 3) matches a filename convention used by NVIDIA's reference Storage service
implementation — trailing `/`, a `.folder` suffix, or a `/.folder/` segment — not a signal the Storage API
contract guarantees. A different Storage service implementation may mark folders another way, or not at all;
verify against your deployment's actual object naming before relying on this check to skip folders.

The supported source extensions are `jpg`, `jpeg`, `png`, `gif`, `bmp`, `tiff`, `tif`, `webp`, and `ico` — exactly the set the worker can decode. `svg`, `heic`, `heif`, and `avif` are deliberately absent because no decoder ships for them; they are skipped here rather than failing later in the worker. The extension is taken from the URL path only: query strings and fragments are stripped first, so `…/image.jpg?x=1` and `…/image.jpg#frag` are eligible, and a `.` inside a directory segment does not count.

The subscription itself excludes directory events — the durable queue's filter groups request only
`omni.storage.created` and `omni.storage.deleted` (see **Filters** above) — so `omni.storage.dir_created` and
`omni.storage.dir_deleted` are not expected to reach this queue. If an event of a type this consumer does not
handle arrives anyway (a misconfigured or non-conformant Notification Service, for example), it is logged as
a warning and acknowledged without work rather than crashing the consumer. A delivery that contains only such
events is acknowledged immediately. Events that cannot be converted from the wire format are logged at `warn`
and acknowledged.

(thumbnail-service-event-consumption-dispatch)=
## Dispatch

When a CREATE event is eligible, the orchestrator publishes one `omni.thumbnail.generate` event for each size
configured in `THUMBNAIL_SIZES`. Each event carries a complete specification — any field the caller didn't set
is filled in from the `THUMBNAIL_*` defaults — and the destination address rendered from the path template.
The orchestrator tries every size, even if an earlier one fails, before deciding the outcome for the whole
delivery:

- **All sizes published successfully** → the delivery is acknowledged.
- **A publish failed for a retryable reason** (the Notification Service was unreachable, the
  `OUTBOUND_REQUEST_TIMEOUT` deadline expired, or the aggregation endpoint isn't configured) → the delivery is
  negatively acknowledged, and redelivery retries every size again. Retrying twice is safe: the worker treats
  a duplicate publish for the same destination as a no-op.
- **A publish failed permanently** (a blank identity or address, a destination the template couldn't render,
  or a specification that fails validation) → the delivery is still acknowledged, so it can't loop forever.

There's no partial result for this path — the orchestrator doesn't report "2 of 3 sizes published."
Per-specification results are only available through the `CreateThumbnails` API. Each publish has its own
deadline (`OUTBOUND_REQUEST_TIMEOUT`, default `30s`), and if it gets an `UNAUTHENTICATED` response, the
orchestrator refreshes its token once and retries that one request. See
{ref}`Event schema <thumbnail-service-notifications-event-schema>` for the exact fields each published event
carries.

(thumbnail-service-event-consumption-acknowledgement-semantics)=
## Acknowledgement semantics

**Acknowledging a StorageEventQueue delivery means the generate work was durably published — not that a thumbnail exists.** Conversion happens later, on WorkerEventQueue, and a conversion failure never reopens or negatively acknowledges the original CREATE. Partial outcomes are observed through per-thumbnail association events, the worker's metrics, and WorkerEventQueue's dead-letter queue, never through a CREATE-level result.

The acknowledgement unit is the **delivery tag**, which covers every event in a delivery. One retryable failure withholds the acknowledgement for the whole delivery; a delivery whose every event was skipped, rejected, or handled is acknowledged as a unit.

(thumbnail-service-event-consumption-redelivery-and-negative-acknowledgement)=
## Redelivery and negative acknowledgement

A retryable failure returns its delivery to the queue with an explicit **negative acknowledgement** on the acknowledgement stream. The broker requeues a negatively acknowledged delivery immediately, so all spacing between attempts at the same work is consumer-side, set by three values that apply to StorageEventQueue and WorkerEventQueue alike:

| Setting | Default | Purpose |
|---|---|---|
| `NACK_REDELIVERY_DELAY_SECS` | `60` | How long a failed delivery's negative acknowledgement is held before it is sent. Keep it far inside the broker's acknowledgement timeout (1800 s in the reference environment): a held delivery is still in flight. |
| `NACK_DEFERRAL_CONCURRENCY_LIMIT` | `10` | Most negative acknowledgements held at once. Held deliveries occupy the prefetch window; past the bound a failure is negatively acknowledged immediately and the early-redelivery gate spaces the retry instead. |
| `NACK_EARLY_REDELIVERY_LIMIT` | `5` | Consecutive early redeliveries of one delivery key before the session is recycled. |

Attempts are spaced per **delivery key** — the storage event's `resource_identity`, falling back to `resource_address` — which survives a redelivery where the delivery tag does not. A redelivery that arrives before its key's next-attempt time is **not reprocessed**: it is negatively acknowledged again (deferred when the bound allows) and logged as `[NACK]`. When one key is redelivered early `NACK_EARLY_REDELIVERY_LIMIT` times in a row, the consumer logs `[CIRCUIT_BREAKER]` and recycles the consume session, achieving spacing by not being connected. Keys idle for an hour are forgotten; that retention window is a fixed constant in the binary, not a configurable value.

Deferrals are owned by the consume loop, not by the processing task, so a saturated consumer still sends them on time. On session end every deferred negative acknowledgement is flushed, and the stream is held open for a 2 s grace so queued acknowledgements reach the wire before it is dropped. If a negative acknowledgement cannot be delivered at all (the acknowledgement stream is broken), the consumer falls back to recycling the session, which redelivers everything unacknowledged.

Each redelivery advances the platform's delivery counter, so the Notification Service's own bounded redelivery and dead-lettering apply unchanged; inspect and replay dead-lettered deliveries with the platform's `ManageDeadLetteredEvents` RPC.

(thumbnail-service-event-consumption-stream-lifecycle-and-reconnects)=
## Stream lifecycle and reconnects

The consumer opens `ConsumeDurableEventsWithAcks` against its queue id and processes deliveries concurrently under a permit bound of `EVENT_PROCESSING_MAX_CONCURRENT` (default `100`; Helm `streaming.maxConcurrent`). A permit is reserved **before** the stream is polled, so at saturation the loop stops reading new deliveries instead of letting the unacknowledged set grow, and shutdown, token-expiry checks, and deferred-acknowledgement deadlines stay observable. Taking the last permit logs `[BACKPRESSURE]`. A quiet, healthy stream is never torn down for silence; `STREAM_TIMEOUT_SECS` bounds provisioning attempts only.

Every 300 s the consumer logs a `[STATS]` line with total and filtered deliveries, active permits against capacity, pending work, stream age, the unacknowledged count, the deferred count, and the time since the last delivery — a frozen total with a rising age is the signature of a stalled subscription.

| Session end | Trigger | Reconnect policy |
|---|---|---|
| Shutdown | SIGTERM / SIGINT | In-flight tasks are given `TASK_CLEANUP_TIMEOUT_SECS` (default `30`) to finish, deferred negative acknowledgements are flushed, the process exits. |
| Token refreshed | The 30 s housekeeping tick finds the service token near expiry; it is refreshed first | Immediate reopen. If the token is still near expiry after refresh, back off. |
| `UNAUTHENTICATED` from the stream | Token refreshed | First occurrence reconnects immediately; a second consecutive one backs off — a refresh is not the fix. |
| Recycle | A negative acknowledgement could not be delivered, or the circuit breaker tripped | Immediate reopen; the broker redelivers everything unacknowledged. |
| `NOT_FOUND` | The queue was deleted underneath the consumer | Re-run provisioning with the configured filter groups, then reopen; no restart needed. Provisioning failure logs an error naming the queue id and backs off. |
| Stream ended, `PERMISSION_DENIED`, other status | — | One error-level log naming the queue and reason (`[RECONNECT]`), then exponential backoff. |

Backoff starts at `RECONNECT_BASE_BACKOFF_SECONDS` (default `5`), doubles up to `RECONNECT_BACKOFF_EXPONENT_CAP` (default `4`) doublings, is capped at `MAX_RECONNECT_BACKOFF_SECONDS` (default `60`; Helm `streaming.maxReconnectBackoffSecs`), and is jittered over the upper half of each interval so replicas do not retry a recovering dependency in lock-step. Every `HEALTH_WARNING_INTERVAL` (default `10`) attempts a warning summarizes the reconnect loop. The consumer channel carries HTTP/2 and TCP keepalive so a dead peer is noticed without treating silence as failure.

(thumbnail-service-event-consumption-storage-deleted-cleanup)=
## Storage deleted cleanup

An `omni.storage.deleted` event triggers the **disassociate sweep** for the deleted source: every `thumbnail_*` association recorded on its address is removed, one `omni.thumbnail.disassociated` event is published per removed entry, and blobs the service itself generated are deleted. The sweep runs under the **service identity**, not a caller token, and is not reachable through the REST or gRPC API — the public disassociate endpoint removes exactly one thumbnail.

The cleanup result drives the delivery's acknowledgement independently of CREATE handling on the same queue:

| Sweep outcome | Acknowledgement |
|---|---|
| The source's metadata could not be enumerated | Negative acknowledgement; redelivery retries the sweep. |
| Any entry failed retryably (storage temporarily unavailable) | Negative acknowledgement. |
| Every failure is permanent (malformed entries, a storage "duplicate batch entry" rejection) | Acknowledged, with an error-level summary naming the permanent failures. |
| All entries removed, or the source had no thumbnail metadata | Acknowledged. |

A source with no associations completes successfully with nothing to do. The two-phase removal, deduplication of thumbnail addresses, and the rule that decides which blobs are deleted are described in {ref}`Storage-deleted sweep <thumbnail-service-associations-storage-deleted-sweep>` and {ref}`Blob deletion policy <thumbnail-service-associations-blob-deletion-policy>`.

(thumbnail-service-event-consumption-see-also)=
## See also

- {ref}`Thumbnail generation worker <thumbnail-service-generation-pipeline-thumbnail-generation-worker>` — the consumer of the `omni.thumbnail.generate` events this page's dispatch publishes, and the second acknowledgement domain.
- {ref}`Thumbnail destination path template <thumbnail-service-destination-path-template-thumbnail-destination-path-template>` — the template that renders each destination and carries the recursion guard.
- {ref}`Association metadata and queries <thumbnail-service-associations-association-metadata-and-queries>` — what the storage-deleted sweep removes and which blobs it deletes.
- {ref}`Service notifications <thumbnail-service-notifications-service-notifications>` — the wire contract of every event consumed and published here.
- {ref}`Authenticating with an OpenID Connect Identity Provider <thumbnail-service-authentication-authenticating-with-an-openid-connect-identity-provider>` — the service identity used to provision, consume, publish, and sweep.
- {ref}`Environment configuration reference <thumbnail-service-configuration-environment-configuration-reference>` — every variable named on this page, with defaults.
- {ref}`Troubleshooting <thumbnail-service-troubleshooting-troubleshooting>` — the queue-id digest check and stalled-stream diagnostics.
