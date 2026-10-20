# Operate Live Edit

> **Applies to:** Live Edit Service (external) · **Released:** 0.1.1 (Developer Guide 1.3.0) · **This repo's snapshot:** not present; the service source, chart, and metric definitions are in ovlive `service/`, not in this repository · **Sources:** the [source skill package](../../NOTICE.md); ovlive `client/CHANGELOG.md` for client behavior

Chart keys, metric names and defaults below come from the source package's description of the released 0.1.1 service and are **unverified in this snapshot**. Before alerting on a metric, confirm its name and labels from the deployed service's `/metrics` output. Confirm the running version first ([capture-a-deployment](../versions/capture-a-deployment.md)).

Boundaries to state in any answer: editing is gRPC-only (no REST or SSE); HTTP `9090` is `/metrics` only; read/stat permission on a base asset is enough to edit its live sessions, so there is no read-only collaborator. Related: [deploy/live-edit.md](../deploy/live-edit.md), [apis/live-edit.md](../apis/live-edit.md), [deploy/discovery.md](../deploy/discovery.md).

## Validation

Port-forward in a separate terminal (plaintext only on the local end of the tunnel):

```bash
kubectl -n <namespace> port-forward service/live-edit-service 50001:50001 9090:9090
```

```bash
grpcurl -plaintext -d '{"service":""}' localhost:50001 grpc.health.v1.Health/Check            # liveness only
grpcurl -plaintext -d '{"service":"nvidia.omniverse.live.v1alpha.LiveService"}' \
  localhost:50001 grpc.health.v1.Health/Check                                                  # readiness: expect SERVING
grpcurl -plaintext -d '{}' localhost:50001 \
  nvidia.omniverse.discovery.capabilities.v2alpha.CapabilitiesService/ListServices            # static; not readiness
curl --fail http://localhost:9090/metrics
grpcurl -plaintext -H "authorization: Bearer ${LIVE_EDIT_AUTH_TOKEN}" \
  -d '{"client_timestamp_microseconds_since_epoch":"0"}' \
  localhost:50001 nvidia.omniverse.live.v1alpha.LiveService/Ping                                # auth round trip
```

Health, reflection, capabilities and metrics answer before the API is fully serving. `Ping` proves connectivity and bearer handling only, not base-asset permission, checkpoint health or edit delivery. Through TLS ingress, drop `-plaintext` and use the ingress host. Validate Discovery from the client network too.

| Check | Proves | How |
| --- | --- | --- |
| Chart smoke tests | Auth, Storage fixture, session create/delete | `tests.enabled: true`, `helm test live-edit-service --logs` |
| Two-client edit | Real collaboration | Two Live Edit clients join one session on a disposable asset; one edits, the other receives |
| usd-content-workspace integration tests 1.0.23 | Released integration coverage | Live Edit tests carry pytest marker `live_edit`; inspect collected and skipped tests ([test/integration-tests.md](../test/integration-tests.md)). A skip is not validation. |
| Backup and recovery | Checkpoints restore | See below |

## Checkpoints and recovery

Checkpoints protect live session state; they never save into the base asset.

1. Edit a disposable layer; confirm checkpoint writes and objects under the backup prefix.
2. Confirm the service identity can read, list, write and delete there, including large uploads that go directly to object storage.
3. Restore into an isolated deployment with a copy of the backups and an empty Valkey. Check sessions and layer content with fresh and reconnecting clients.
4. Measure recovery time and recovered content against requirements.

After a restore, clients must reload layers from empty state (`object_id: 0`, `sequence_num: 0`); a reconnecting client can hold content newer than the backup. Keep `layer.catchup_snapshot_enabled: true` and test the client's reload path. ovlive's exactly-once `operation_id` records do not survive a backing-store loss followed by a restore, so a re-send after a restore can apply twice (vendored proto comments).

- Do not delete individual backup objects or apply lifecycle rules to the backup prefix.
- One backup prefix per deployment; all replicas share one checkpoint configuration.
- `LocalDisk` is not shared; never use `InMemory` for recovery.

## Access changes

| Setting | Default | Effect |
| --- | --- | --- |
| `authorization.cache_ttl_secs` | 600 | Allowed decisions cached |
| `authorization.cache_negative_ttl_secs` | 30 | Denied decisions cached |

Streams check access when they open; revoking a permission does not disconnect existing sessions or subscriptions. An inference from that behavior, not a documented procedure: to cut off a user quickly, revoke in Storage and roll the Live Edit pods through the release workflow so streams reopen and are re-checked; whether the decision cache survives a restart is unverified.

## Scaling and capacity

| Setting | Default | Effect |
| --- | --- | --- |
| `replicaCount` / `autoscaling` | 3; HPA 3–10 on CPU and memory (80%) | Replicas share Valkey; no affinity |
| `podDisruptionBudget.minAvailable` | 2 | Keep consistent with a fixed replica count |
| `valkey.subscriber_pool_size` | 256 | Concurrent layer subscriptions per replica; each is a Valkey connection |
| `valkey.pool_size` | 64 | General connections; does not raise the subscription limit |
| `layer.max_size` | 104857600 | Largest upload |
| `valkey.layer_stream_maxlen` | 4096 | Retained updates per layer; lower saves memory but shrinks the incremental-backup window |
| `shutdown.timeout_secs` / `shutdown.overall_budget_secs` | 5 / 30 | Drain, then cleanup budget |
| `terminationGracePeriodSeconds` | 40 | Longer than both shutdown budgets combined |

- Existing streams stay on the pod that accepted them; adding replicas does not move them.
- Watch subscription capacity separately from CPU and memory autoscaling. The released guide says excess subscriptions wait for capacity; an earlier source said immediate `UNAVAILABLE`. Do not promise a refusal status or metric without checking the deployed artifact.
- More replicas add connections and load to the same Valkey. One very busy layer is bounded by one Valkey shard's CPU.
- See also [operate/scalability.md](scalability.md#live-edit).

## Metrics

Prometheus metrics are always served on `:9090/metrics`. Set `serviceMonitor.enabled: true` only when the Prometheus Operator CRDs exist.

| Metric (source package) | Watch for |
| --- | --- |
| `live_edit_valkey_pool_in_use{pool="subscriber_pool"}` | Subscriptions in use versus `subscriber_pool_size` |
| `live_subscribe_regime_total`, `live_subscribe_catch_up_duration_seconds` | Bootstrap versus reconnect load; catch-up latency |
| `live_edit_valkey_errors_total`, `live_edit_valkey_reconnects_total` | Valkey connectivity or auth trouble |
| `live_edit_checkpoint_writes_total{result="error"}` | Failed backups |
| `live_edit_checkpoint_oldest_pending_age_seconds` | Backups falling behind |
| `live_edit_checkpoint_layers_never_written` | Layers with no backup |
| `live_edit_checkpoint_adapter_redirect_total{verb="put",outcome="target_failed"}` | Large backups not reaching object storage |
| `live_edit_checkpoint_tail_total{outcome="refused_trimmed"}` | Edits aged out of Valkey before backup |
| `live_edit_service_token_available` | Checkpoint service token missing or expired |

OTLP export (each signal needs its own endpoint; these keys differ from the Notification services' `telemetry` block):

```yaml
observability:
  metrics:
    enabled: true
    endpoint:
      grpc: "http://otel-collector.observability.svc.cluster.local:4317"
  tracing:
    enabled: true
    endpoint:
      grpc: "http://otel-collector.observability.svc.cluster.local:4317"
```

Client side, ovlive exposes typed status and a diagnostic snapshot (`layer_writes`, `apply_lag`) described in its [diagnostic schema](https://github.com/NVIDIA-Omniverse/ovlive/blob/main/client/docs/ovlive-diagnostic-schema.md).

## Configuration changes

- Keep complete values files in the release workflow (for example GitOps). Avoid one-off `helm upgrade --set`.
- ConfigMap-backed changes trigger a rolling restart; clients must reconnect.
- Rotating a Secret consumed as an environment variable (such as `live-edit-env`) does not restart pods; schedule the restart. Token files are re-read without a restart.
- Validate before applying: `helm lint` and `helm template` with every values file in its original order.
- Roll back through the chart release (`helm rollback` or the workflow), not by editing the pod template, then re-run the smoke tests. Rollback does not restore Valkey data.

## Troubleshooting

| Symptom | Likely cause | Fix |
| --- | --- | --- |
| Pod exits at startup with default values | `checkpoint.enabled: true` with `adapter: Disabled` | Configure `StorageApi`/`LocalDisk`, or disable checkpoints with AOF-backed Valkey |
| Startup refused with checkpoints off | Valkey reports `appendonly no` | Enable AOF or checkpoints; never switch to an evicting policy |
| Liveness `SERVING`, readiness `NOT_SERVING` | Valkey connectivity/auth/TLS, startup still running, or service token missing | Read logs; Storage and User Info reachability are not part of readiness |
| Redis token valid, pod unready, checkpoints off | `serviceIdentity.accessTokenFile` also needs the Storage token | Keep a valid token at `checkpoint.credentials.accessTokenFile.path` |
| Slow first start | Startup within the 280 s probe budget | Raise Helm `--timeout`; do not weaken probes |
| `INTERNAL` with opaque tokens | JWT claim decoding cannot read them | Enable `userInfo` or use JWTs |
| `helm upgrade --install` fails with tests enabled | Smoke tests run as hooks | Read the test pod logs; fix the test principal's access to `tests.storageApiResourceBase` |
| Absent from Discovery or unreachable | Annotation, `grpc-port: "50001"`, visibility | Check annotations, operator logs, `/api/v2/services` (schema 2); test the advertised endpoint from clients |
| Path RPCs `INTERNAL`, Storage logs 405 | Storage endpoint lacks `HEAD` by address | Use an endpoint that supports `HEAD`, or `protocol: grpc` with the Storage gRPC endpoint |
| Small backups work, large never publish | No egress to the presigned host | Allow DNS, TLS and HTTPS to the bucket host; check `redirect_total{outcome="target_failed"}` |
| AWS IAM auth fails | `clusterHost` is an FQDN, non-cluster URL, or missing grant | `clusterHost` = cluster name; `rediss-cluster://`; check IAM/ACL |
| Subscriptions wait or time out | Subscriber capacity, load, Valkey latency, ingress timeout | Inspect active subscriptions and dependencies; tune measured capacity |
| Writer never sees its own edit | Same non-empty `client_id` | Expected; test with two IDs |
| Streams drop during rollout | Pods draining | Clients resubscribe from the last applied cursor and treat `OK` stream end as reconnect |
| Writes fail as Valkey fills | `noeviction` at `maxmemory` | Add memory or reduce retained data; keep `noeviction`; never `FLUSHALL` |

```bash
kubectl -n <namespace> logs deployment/live-edit-service -c live-edit-service --tail=100
kubectl -n <namespace> describe pods -l app.kubernetes.io/instance=live-edit-service
```
