# Operate notifications

> **Applies to:** Event Aggregation Service, Event Consumer Service, RabbitMQ · **Released:** Aggregation 1.6.0, Consumer 1.11.0, RabbitMQ 4.1.3 (chart 99.3.0) · **This repo's snapshot:** Aggregation 1.7.0, Consumer 1.12.0 (`pyproject.toml`, `Chart.yaml`) · **Sources:** [consumer `README.md`](../../../../notifications/reference/event-consumer-service/README.md) (Deployment sections), [consumer `helm/README.md`](../../../../notifications/reference/event-consumer-service/helm/README.md), [aggregation `README.md`](../../../../notifications/reference/event-aggregation-service/README.md), both `values.yaml` and `CHANGELOG.md` files, the [source skill package](../../NOTICE.md)

Use this page to upgrade, size, roll out, observe, and troubleshoot a running notification stack. Confirm the versions actually deployed first ([capture a deployment](../versions/capture-a-deployment.md)), read that version's `values.yaml`, and keep existing auth, ingress, and broker overrides. Initial install is in [deploy](../deploy/notifications.md); wire semantics are in [API](../apis/notifications.md). Stack-wide monitoring, runbooks, and known issues: [monitoring](monitoring.md), [runbooks](runbooks.md), [known issues](known-issues.md), [upgrades](upgrades.md).

## Upgrade path

| Step | What changes | What you must do |
|---|---|---|
| Consumer → 1.10.0 | Durable queues become quorum queues (RabbitMQ 4.x); DLQ; `ConsumeDurableEventsWithAcks`; `ManageDeadLetteredEvents`; REST `POST /api/v1beta/events/ack-durable` removed; REST durable SSE no longer carries `session_id`/`delivery_tag`; `ConsumeDurableEvents` deprecated | Upgrade the broker to 4.x first. Move clients that acked over REST to gRPC with-acks. An existing classic queue cannot be redeclared as quorum. |
| Aggregation → 1.6.0 | Auth before validation (a bad token plus a malformed body is now `401`, not `400`/`422`); empty principal claim refused; JWKS outage is `500`; `413` body limit; `shutdown` block and `terminationGracePeriodSeconds: 60`; `observability` block; Discovery `ListServices`; `503` removed from the REST spec | Check clients that depend on error ordering. Check `OMNI_EVENTS_PRINCIPAL_ID_CLAIM` resolves non-empty for every caller. Overrides with a grace period below 30 s now fail rendering. Pod teardown takes ~15 s longer. |
| Consumer → 1.11.0 | Deployment-wide `queue_id` (`event-queue:<queue_id>`), ensure-exists create, `created` field, `nack_delivery_tags`, filter-update acks, `Last-Event-ID`, `policyHook` (idle expiry 24 h), `shutdown` block, consumer-loss errors, `queue_ref` in non-durable telemetry, `OMNI_EVENTS_CONSUMER_TIMEOUT_MS` removed | Create the management Secret and set the three `policyHook.managementApi.*` values **before** `helm upgrade`. Migrate identity-prefixed durable queues (below). Move dashboards keyed on `queue_id` (non-durable) or the `UNKNOWN` gRPC bucket. Replace `shutdown.preStopDelaySeconds` with `settleDelaySeconds`. Re-check tuned grace periods against the three-term bound. |
| Aggregation → 1.7.0 (snapshot) | Per-event limits on both APIs: ≤ 1000 events, `message` ≤ 65536 JSON bytes, `event_type` ≤ 256, `resource_id` ≤ 2048; refusal is `422` / `INVALID_ARGUMENT` for the whole batch | Find publishers with large messages before upgrading. The source package's "keep messages under 100 KB" guidance exceeds this cap. Storage Service `notificationClient.batchSize` (100) is within limits. |
| Consumer → 1.12.0 (snapshot) | Bug and security fixes only (see [component map](../components/event-consumer-service.md#released-1110-versus-snapshot-1120)) | Non-durable storage subscribers may start receiving events for new and empty folders they were previously not sent. |

Chart values did not change between released 1.6.0/1.11.0 and snapshot 1.7.0/1.12.0. In the snapshot, both services' generated OpenAPI documents (`/openapi.json` and `openapi.yaml`) move the internal API-governance ID from the top-level `tags` list into an `x-` extension under `info`; if an API catalog or scanner of yours reads that tag, update it. Run `helm lint` and inspect `helm template` output with the same ordered `-f` files before every upgrade.

### Migrating identity-prefixed durable queues (to Consumer ≥ 1.11.0)

Queues created by ≤ 1.10.x carry an identity-derived broker name and are unreachable after the upgrade; consuming returns `NOT_FOUND`. Recreating is not replay. A tested procedure is not in this repo; the source package's plan, consistent with the CHANGELOG:

1. Inventory old queues, their filters, and their message counts; choose deployment-unique new `queue_id` values and record the client mapping.
2. Pause publishers; drain old queues with the old service version; resolve any dead letters.
3. Upgrade, verify the `event-queue-limits` policy, then call `CreateDurableQueue` with each new `queue_id` and its filters; verify quorum type and bindings.
4. Point clients at the new ids, then resume publishers. Plan rollback around which broker names each version understands. Never delete a queue that still holds needed messages.

## Capacity and scaling

### Aggregation

Publishes hold a channel for milliseconds. Chart defaults: `rabbitmq.channel_pool_size: 25`, `channel_pool_min_size: 5`, `channel_client_limit: 1`, `channel_overflow_enabled: true` (packing threshold, not a concurrency cap; one channel error can fail several in-flight publishes), `channel_wait_ms: 500`. For strict isolation set overflow off and raise the pool; publishes then wait up to `channel_wait_ms` and fail with `success: false`. REST bodies are capped by `rest.maxRequestBytes` (4194304); keep it well under the broker `max_message_size` (16 MiB default) because a 4 MiB body can publish as ~12 MiB. gRPC has its own fixed 4 MiB inbound cap. Scale with `replicaCount` or `autoscaling.*` (HPA on CPU, off by default).

### Consumer admission tiers (per pod, non-durable)

| Tier | Set by | Default | Effect |
|---|---|---|---|
| Soft threshold | `rabbitmq.channel_pool_size` × `channel_client_limit` | 100 × 8 = 800 | Readiness and autoscaler signal; refuses nobody. Pod goes NOT_SERVING once free slots < `readiness_min_available_channels` (80), so 720 usable in the signal |
| Shed threshold | `rabbitmq.max_re_routable_streams_per_pod` | null → midpoint 2,071 | Refuses re-routable streams (all REST/SSE, gRPC via proxy with `X-Via-Proxy`) |
| Memory cap | `broker_stream_memory_budget_mib` (1024, fleet-wide) ÷ divisor, prefetch `channel_prefetch_count` (15) | 3,342 (fleet 33,429 ÷ 10) | Refuses every stream: `503` / `RESOURCE_EXHAUSTED` |

The divisor is `channelPoolAutoscaling.maxReplicas` (10) with autoscaling on, else `replicaCount`. Raising `maxReplicas` **lowers** each pod's cap and rolls every service pod (checksum on the settings ConfigMap). When the cap binds, raise broker RAM and the budget together, or lower prefetch; adding pods does not help. A single-replica fleet should set `max_re_routable_streams_per_pod` equal to the cap. Raising prefetch toward 500 at client limit 8 inverts the band (cap 118 < 800). `max_non_durable_streams_per_pod` overrides the computed cap; use only with measured capacity.

Durable operations and DLQ browsing use a separate strict pool (`durable_channel_pool_size: 100`, `durable_channel_client_limit: 1`); a full durable pool is a configuration error. The autoscaler does not measure it.

Autoscaler (`channelPoolAutoscaling.*`): floor `minReplicas: 3` (re-applied each poll), ceiling 10; scale up when fleet available < `minAvailableCapacity` (800); scale down only when the 300 s trailing minimum ≥ 800 + 720, after 600 s post-scale-up hold and 1800 s cooldown. `kubectl scale --replicas=0` stops the fleet and is not reverted; a fleet with no reporting pods is left alone. `GET /api/v1beta/metrics/channel-pool` reports one pod in stream slots; `channels_in_use` can exceed `channels_total` in the overflow band.

### Broker sizing

Per node: `RAM_node >= (B + (S / n) * (P * m + C)) / W`, with baseline `B` (measure under load), fleet streams `S`, nodes `n`, prefetch `P` (15), per-held-message `m` ≈ 1.5–1.8 kB measured, per-stream channel share `C` ≈ 5 kB, high-watermark `W` (0.6). Add the reconnect-grace orphan backlog: `orphaned queues × matching event rate × grace seconds × per-message cost` (example in the README: ~2 GiB for 12,000 churned streams at 5 events/s, 20 s). Keep `broker_stream_memory_budget_mib` below real broker memory to leave room for it and for durable backlogs. Reaching ~12,000 fleet streams: `maxReplicas: 17`, or `channel_client_limit: 16` at 10 pods. A memory alarm blocks publishers (aggregation), not consumers.

## Graceful shutdown and rollouts

Both charts enforce `terminationGracePeriodSeconds >= settleDelaySeconds + drainWindowSeconds + 10` at render time.

| | settle | drain | grace | Behavior |
|---|---|---|---|---|
| Aggregation | 15 | 5 | 60 | Serves normally through settle and drain; HTTPProxy has no retry, so the settle is the only de-route cover |
| Consumer | 15 | 30 | 60 | After settle, readiness NOT_SERVING and new non-durable streams refused (`503`/`RESOURCE_EXHAUSTED`, retried by ingress onto other pods); in-flight streams drain |

Idle REST/SSE streams end at their next liveness poll (up to ~17 s), which the 10 s term does not cover; raise the grace period if clean SSE termination matters. On the consumer's proxied path (envoy Service publishes not-ready addresses) a short residual window remains that only `httpProxy.unary.retryPolicy` and `httpProxy.streaming.retryPolicy` cover. Clients must reconnect after any pod termination: non-durable within 20 s to keep their backlog, durable with the same `queue_id`. SIGKILL during teardown requeues unacked durable deliveries and spends delivery count.

## Durable-queue bounds and recovery

The consumer's hook Job (`<release-fullname>-queue-policy`) applies policy `event-queue-limits` (pattern `^event-queue:`, apply-to `quorum_queues`, priority 0) on every install and upgrade. Defaults: `deliveryLimit: 10`, `messageTtlMs: 86400000`, `maxLength: 100000`, `overflow: drop-head`, `expiresMs: 86400000`, `consumerTimeoutMs: null`.

- Fail-closed: if the policy cannot be applied, the release fails — after the new pods may already be running. Check the Job before starting durable workloads. Retries: 3 per API call, `backoffLimit: 1`, `activeDeadlineSeconds: 180`.
- The Job cross-checks `managementApi.vhost` against the vhost in `rabbitmq_uri`.
- An unchanged policy is not rewritten, because applying a policy resets every matched queue's idle clock. After the first apply, the first reap happens a full `expiresMs` later, even for long-abandoned queues.
- Only one regular policy applies per queue; a second matching policy displaces this one rather than merging. Keep every bound in it.
- `policyHook.enabled: false` stops updates but does not remove an already-applied policy (clear it with `rabbitmqctl clear_policy -p <vhost> event-queue-limits`). If it was never applied, durable queues have no length, TTL, or expiry bound (only RabbitMQ 4.x's default delivery limit of 20); apply an equivalent policy yourself.
- Keep `expiresMs >= messageTtlMs`. `null` disables reaping; `0` is invalid. Expiry discards remaining messages; they are not dead-lettered.
- `consumerTimeoutMs` (null = broker default, 30 min on 4.1): size above `durable_consumer_prefetch_count` (20) × worst-case per-message time; the chart refuses < 60000, RabbitMQ discourages < 300000, enforcement ticks once a minute.

When a client reports `NOT_FOUND` on a durable consume: confirm it is not a wrong endpoint, vhost, or auth failure; then the client recreates with its stored `queue_id` and `filter_groups`. Events published while the queue was gone are lost; reconcile from the source system. Queue permissions do not prove which application owns an id; the service logs `Durable queue created: id=… principal=…` and `Durable queue attached (existing): …` for every create call.

Nack loops: RabbitMQ requeues a nacked message immediately, and every nack, disconnect, or channel close spends the same lifetime budget. A tight nack loop dead-letters work in under a second. Clients must back off before nacking.

## Dead-letter triage

DLQ: exchange `events.dlx`, queue `events.dlq`, message TTL 14 days (`OMNI_EVENTS_DEAD_LETTER_QUEUE_MESSAGE_TTL_MS`), max length 1,000,000 with `drop-head` (`OMNI_EVENTS_DEAD_LETTER_QUEUE_MAX_LENGTH`). The DLQ is global, and capture into it is at-most-once; overflow or expiry there is permanent loss. These are queue arguments applied at declaration only. The chart does not template them, and adding them to the `envSecretName` Secret does not inject them (the Deployment reads only `rabbitmq_uri`/`redis_uri` from it). Changing them later logs one `Dead-letter queue argument mismatch` ERROR per connect and keeps the live bounds. To lower a bound, apply a separate policy on `^events\.dlq$` without `overflow`; to raise one, drain and delete the DLQ in a maintenance window (commands in the consumer README → Retuning the dead-letter queue's bounds).

Triage uses gRPC `nvidia.omniverse.notifications.consumer.v1beta.EventConsumerService/ManageDeadLetteredEvents` only (no REST), with `event-consumer-service:inspect-and-replay-all-dlq` — a deployment-wide grant that exposes every dead letter. Open the stream with `{}`, inspect `reason` and `origin_queue`, then act:

| Action | Result |
|---|---|
| `ACTION_REPLAY` | Republishes to the recorded origin queue; removes the DLQ copy after broker confirm (`OUTCOME_REPLAYED`). Confirms publication, not downstream success; duplicates possible. `OUTCOME_ORIGIN_QUEUE_DELETED` / `OUTCOME_REPLAY_FAILED` leave the copy in place |
| `ACTION_DISCARD` | Permanently removes it |
| `ACTION_LEAVE` | Requeues in the DLQ. Each leave spends the copy's delivery count. The DLQ is a quorum queue with no dead-letter exchange and no delivery-limit argument, so RabbitMQ 4.x's default quorum delivery limit (20) can eventually drop a message that is left repeatedly (broker behavior; not tested in this repo) |

Fix the consumer bug before replaying `REASON_DELIVERY_LIMIT`; for `REASON_EXPIRED` and `REASON_MAXLEN`, fix backlog or capacity and judge whether old events still matter. Replay cannot be redirected; a recreated origin queue must still belong to the intended application. Delivery tags are valid only on the live stream. Example with `grpcurl` (supply your own TLS and token handling):

```bash
grpcurl -H "authorization: Bearer $OPERATOR_TOKEN" \
  -import-path notifications/api/protos -proto nvidia/omniverse/notifications/consumer/v1beta/event_consumer.proto \
  -d @ grpc.event-consumer.<domain>:443 \
  nvidia.omniverse.notifications.consumer.v1beta.EventConsumerService/ManageDeadLetteredEvents <<'EOF'
{}
{"actions":[{"deliveryTag":"<tag-received-on-this-stream>","action":"ACTION_LEAVE"}]}
EOF
```

A heredoc sends both messages at once, before any tag is known; for real triage use an interactive client such as the Python example in [`docs/consuming-events.md`](../../../../notifications/reference/event-consumer-service/docs/consuming-events.md) → Managing the Dead-Letter Queue (Operators). The `grpcurl` command shape is not exercised in this repo.

## Observability

Enable `observability.metrics` and `observability.tracing` independently, each with `endpoint.grpc` (OTLP/gRPC; `endpoint.http` is ignored). Either enabled signal overrides the legacy `telemetry` block, and logs ship whenever either is on (to the tracing endpoint, else the metrics endpoint). To turn export off, disable `telemetry.enabled` too. `tracing.samplingRatio` is not honored.

Signals worth alerting on:

- Publish results with `success: false` (HTTP status alone is not enough).
- Auth refusals: one `WARNING` `Rejecting request with an unresolvable token: …`; gRPC refusals count under `UNAUTHENTICATED`, JWKS outages under `INTERNAL`; REST auth refusals produce no span. Aggregation also logs `413` refusals at `WARNING`, with no span and no CORS headers.
- Consumer: `storage listing did not authorize a directory…` at `WARNING` (misrouted `permissions.storage_permissions.endpoints`), `Dead-letter queue argument mismatch`, consumer-loss errors.
- Non-durable streams are identified as `queue_ref` (`sha256-<12 hex>` of the token) in logs, spans, metrics, and the access log; durable streams keep `queue_id`.
- Per-pod stream counts against soft threshold, shed threshold, and cap; DLQ depth; durable queue depth against `maxLength`.

Readiness reflects capacity and drain; liveness (`liveness` gRPC service) must never be tied to load.

## Verification

After each rollout: both Deployments ready; the queue-policy Job succeeded and its log shows the policy applied or unchanged; the effective broker policy matches values; `GET /api/v1beta/health` on each Service; `helm test` (4 aggregation, 6 consumer smoke tests, with `tests.auth` and the needed permissions; disable `tests.runRestTests` if REST is off); an end-to-end publish/receive ([deploy → install and verify](../deploy/notifications.md#install-and-verify)). Test separately, in staging: nack and redelivery to the DLQ, non-durable reconnect within and after 20 s, durable expiry and recreate, and each DLQ action. Conformance suites do not cover DLQ management.

## Troubleshooting

| Symptom | Check |
|---|---|
| Release fails on consumer upgrade | `policyHook.managementApi.*` missing, Secret absent, credential lacks `policymaker`/vhost access, management URL unreachable, vhost mismatch with `rabbitmq_uri`, or `deliveryLimit`/`consumerTimeoutMs` refused by the render guard |
| Render fails on `terminationGracePeriodSeconds` | Grace below settle + drain + 10, or non-integer |
| Publish `200` but nothing arrives | `result.success`; publisher and consumer on the same broker and vhost; filters (`EQ` on `""` is an error; lazy vs greedy); non-durable storage events filtered by the caller's storage listing permission; consumer connected before the publish |
| New `401`s after upgrade | Principal claim empty or absent in some tokens; `jwks_uri` redirect (PyJWT ≥ 2.15); clock-expired token with `jwt` |
| Same caller authorized differently on the two services | `openId.principalIdClaim` or `principalSource` differ between charts |
| Durable `NOT_FOUND` | Reaped after `expiresMs` idle, deleted, or pre-1.11 identity-prefixed queue; recreate with stored id and filters |
| Streams end with `INTERNAL` "broker cancelled" | Consumer timeout too low, broker leader change, channel error; reconnect, check `consumerTimeoutMs` sizing |
| `503` / `RESOURCE_EXHAUSTED` on consume | Pod draining, shed threshold, or memory cap; see tiers; do not add pods for a binding cap |
| Streams drop every 60 s–4 min | L4 load balancer idle timeout; client reconnect logic |
| DLQ growing | Poison messages (`REASON_DELIVERY_LIMIT`), tight nack loops, slow consumers (`REASON_MAXLEN`/`EXPIRED`) |
