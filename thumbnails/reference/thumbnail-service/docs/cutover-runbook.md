# Cutover runbook: Python task queue → Rust `task-worker`

Retiring `thumbnail-task-queue` and bringing up `task-worker` happen in one
window. There is no dual-write period: production never runs Celery enqueue and
generate publishing against the same CREATE stream, because two dispatchers for
one source produce two thumbnails and no way to tell which association won.

This is the operator-facing sequence. The design rationale is in
`openspec/changes/rust-thumbnail-task-worker/design.md`; the worker's own
behavior is in `helm/thumbnail-task-worker/README.md`.

## What changes

| Before | After |
|---|---|
| `server` HTTP-POSTs each size to `thumbnail-task-queue` | `server` publishes one `omni.thumbnail.generate` event per size |
| Celery workers convert with Pillow | `task-worker` converts with the `image` crate |
| RabbitMQ is the work queue | Notification WorkerEventQueue is the work queue |
| Postgres holds task rows | no task store; the worker exposes counters |
| `DELETE /thumbnails` (which never existed) removed blobs | the service deletes blobs against storage directly |

One deployment becomes two, from two charts, with two durable queues and two
acknowledgement domains. See `helm/DEPLOYMENT_ARCHITECTURE.md`.

## Prerequisites

None of these are part of the deploy; all must be true before it starts.

1. **Notification `event_permissions`** — an entry for
   `omni.thumbnail.generate` in the file named by
   `OMNI_EVENTS_PERMISSIONS_YAML_FILE_PATH`, plus the matching policy on
   `EventResourceId`. Durable consume of a non-storage event type runs the
   fine-grained check, not the storage `consume-all-*` actions. **This is a
   change to the Notification deployment**, not a grant that can be made from
   this side, so it has the longest lead time of anything here.
2. **IAM grants** on the shared service-account principal — `server` and
   `task-worker` run as one identity, so each grant is made once:
   - `event-aggregation-service:publish-event` on `EventType:omni.thumbnail.generate`
   - `event-consumer-service:create-durable-queues`
   - `event-consumer-service:consume-durable-queues`
3. **WorkerEventQueue's id chosen**, distinct from the server's storage-CREATE queue id.
   Queue ids are namespaced per principal, so a distinct bare id is enough — but
   it is not optional. Both charts refuse to render without one.
4. Optionally set **`GENERATED_THUMBNAIL_PRINCIPAL_IDENTITY`** on the service chart to the
   `task-worker` writer principal. This guard is still inert everywhere because storage does not
   propagate a writer principal, so template-output shape matching against
   `THUMBNAIL_PATH_TEMPLATE` is the only recursion guard actually running. Do not weaken the
   template.

## Sequence

The ordering constraint that matters is step 2 before step 3. `PublishEvent`
succeeds whether or not any queue is bound to the event type, so a
generate-publishing `server` in an environment without WorkerEventQueue acknowledges
every CREATE for work that is silently dropped — no error, no retry, no
thumbnails.

**1. Deploy `task-worker`.** It self-provisions WorkerEventQueue on startup.

```bash
helm upgrade --install thumbnail-task-worker helm/thumbnail-task-worker \
  --namespace "$NS" --values <env-values>.yaml
kubectl --namespace "$NS" rollout status deployment/thumbnail-task-worker
```

**2. Verify WorkerEventQueue exists and is being consumed.** This is the gate, and it is
scripted so it cannot be eyeballed:

```bash
scripts-notifications/verify-generate-queue.sh \
  --queue-id "$WORKER_QUEUE_ID" --namespace "$NS"
```

It checks the queue exists with a generate filter, that its id is not the
server's, and that at least one worker replica is Ready — which for this worker
means its consume stream is established, not merely that the process started.

**3. Deploy `server` with generate dispatch.**

```bash
helm upgrade --install thumbnail-service helm/thumbnail-service \
  --namespace "$NS" --values <env-values>.yaml
kubectl --namespace "$NS" rollout status deployment/thumbnail-service
```

**4. Smoke-test the whole path** before taking anything down — see below.

**5. Take down the Python task queue.** API Deployment, Celery worker
Deployment, and its chart. In-flight Celery work at this moment is abandoned;
step 7 recovers it.

**6. Confirm there is no dual dispatch.** Nothing should be reaching the
task-queue endpoint, because no code path reads it any more — the variable is
gone from the config surface entirely. Confirm the Python Deployments are gone
and the worker is the only thing converting.

**7. Backfill anything stranded.** CREATEs acknowledged after a successful
Celery enqueue, whose tasks never ran before the cut, produced nothing:

```bash
scripts/create-missing-thumbnails.sh --folder "$WATCHED_PREFIX" --token "$TOKEN"
```

## Smoke test

Prove the whole path, not just that pods are Running. Upload one image to a
watched prefix and assert the thumbnail appears and is associated:

```bash
# 1. The worker is consuming.
kubectl --namespace "$NS" get deployment thumbnail-task-worker \
  -o jsonpath='{.status.readyReplicas}'

# 2. Baseline the success counter.
make worker-metrics | grep 'outcome="success"'

# 3. Upload a source image to a watched prefix, then wait.

# 4. The counter moved, and nothing was permanently rejected.
make worker-metrics | grep -E 'outcome="(success|permanent_failure)"'

# 5. The association exists — the real end-to-end assertion, because it can
#    only be true if convert, upload, and the associate RPC all succeeded.
curl -sS -G -H "Authorization: Bearer $TOKEN" \
  --data-urlencode "association_target_resource_address=$UPLOADED_ADDRESS" \
  "$SERVICE/v1alpha/thumbnails" | jq .
```

Cutover is not successful until step 5 returns a thumbnail. Steps 1–4 can all
pass while association is broken.

Also worth doing once in stage: **verify idempotent redelivery.** Publish a
duplicate generate event for a destination that already exists and confirm the
second delivery succeeds rather than failing. The worker takes no lease — two
replicas may both convert the same destination — and correctness rests on the
upload overwriting and the association being idempotent.

## Rollback

Rollback is a coordinated restore of both sides, not a single revert. The
service and worker share the generate contract, so a `server` that publishes
generate events with no worker consuming is as broken as the reverse.

```bash
# 1. Stop new work entering WorkerEventQueue.
helm rollback thumbnail-service <previous-revision> --namespace "$NS"

# 2. Bring the Python task queue back up.
#    Its chart lives in the thumbnail-task-queue repository.

# 3. Scale the worker down only after WorkerEventQueue has drained (see below).
kubectl --namespace "$NS" scale deployment/thumbnail-task-worker --replicas=0
```

**What happens to generate messages already on WorkerEventQueue.** They are durable and
survive the rollback — they do not disappear when the publisher is reverted.
Two options, and the choice depends on how long the rollback is expected to
last:

- **Drain them first** (preferred). Leave the worker running until
  `thumbnail_worker_unacknowledged_deliveries` reaches zero and the success
  counter stops moving, then scale to zero. Those thumbnails get produced and
  associated normally, and the rolled-back service reads them happily —
  associations are the same shape either way.
- **Scale the worker down immediately.** The messages stay queued. They will be
  delivered whenever a worker returns, which may be the next roll-forward; if
  that is far enough away, they hit the queue's message TTL or maximum length
  and dead-letter. Sources whose thumbnails were lost this way are recoverable
  with `scripts/create-missing-thumbnails.sh` after the roll-forward.

Do **not** delete WorkerEventQueue during a rollback. Recreating it later does not
recover the messages it held, and a deleted queue receives nothing published
before it is recreated.

Blob deletion does not need reverting: the rolled-back service asks the task
queue to delete blobs, which never worked, and the rolled-forward service
deletes them against storage. Neither loses data; the old path simply leaves
orphans.

## Operator surface that goes away

Retiring Postgres and Celery removes per-task queryability. This is accepted,
and each capability has a replacement:

| Retired | Replacement |
|---|---|
| `GET /tasks/{task_id}` | No per-item status. Aggregate outcome counters on the worker's `/metrics`; the association itself is the durable record of success |
| `GET /tasks` | WorkerEventQueue depth for backlog; `thumbnail_worker_unacknowledged_deliveries` for in-flight retries |
| `DELETE /tasks/{task_id}` | No cancellation. Work is bounded by the queue and the in-flight cap instead |
| Flower | `make worker-logs`, `make worker-metrics` |
| `/celery/stats` | `thumbnail_worker_outcomes_total`, `thumbnail_worker_failures_total{class=...}`, `thumbnail_worker_conversion_milliseconds_total` |
| Poison-task rows in Postgres | Error-level log naming the destination and failure class before every permanent rejection; WorkerEventQueue's DLQ via `ManageDeadLetteredEvents` for exhausted retryables |

The one real loss is **per-item lookup**: you can no longer ask "what happened
to this specific file". You can ask what happened in aggregate, and you can find
an individual failure in the logs by destination address. That trade was made
deliberately — see Decision 13 in the design.

## Downstream consumers outside this repository

The scaling harnesses under
`observability/usd-storage-api-observability/harnesses/thumbnails/` break at
cutover. They are not in this repository and are not fixed by this change.

`run_thumbnail_scaling.py` uses the Postgres `tasks` table as its completion
meter — `SELECT status, count(*) FROM tasks WHERE input_data LIKE '<RUN_TAG>'` —
and scales the `thumbnail-task-queue-worker` Deployment. Both disappear.

Replacements, for whoever owns them:

- **Scaling target**: `kubectl scale deployment/thumbnail-task-worker`, and
  `MAX_CONCURRENT_CONVERSIONS` for per-replica concurrency. Note the two are
  sized together against the pod memory limit; changing replicas alone no longer
  tells the whole story.
- **Completion meter**: the delta in
  `thumbnail_worker_outcomes_total{outcome="success"}` across a run, or counting
  `omni.thumbnail.associated` events.
- **The gap**: neither replacement supports the harness's `RUN_TAG` filter. The
  worker counters are process-aggregate, so a run can only be measured as a
  delta on an otherwise-quiet cluster, and the enqueue precondition check ("one
  upload must produce exactly one task row") has no equivalent at all. If
  per-run attribution is genuinely needed, it wants association events filtered
  by source prefix rather than a metrics delta.

Decide before the cutover whether these harnesses get ported or retired.
Leaving them in place means they fail confusingly rather than loudly.
