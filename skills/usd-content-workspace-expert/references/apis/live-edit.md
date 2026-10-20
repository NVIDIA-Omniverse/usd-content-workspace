# Live Edit API (LiveService)

> **Applies to:** `nvidia.omniverse.live.v1alpha.LiveService` · **Released:** Live Edit Service 0.1.1 (Developer Guide 1.3.0) · **This repo's snapshot:** no service or proto in this repository. A vendored copy of the proto is in [ovlive](https://github.com/NVIDIA-Omniverse/ovlive) at `client/source/live/protos/nvidia/omniverse/live/v1alpha/live.proto`, copied from the service repository's `proto/nvidia/omniverse/live/v1alpha/live_service.proto` at main commit `13a3099`, with additions not yet in the service (see below) · **Sources:** the vendored proto and its `PROVENANCE.md`, `client/CHANGELOG.md` in ovlive; the [source skill package](../../NOTICE.md)

The authoritative wire schema is the one the deployed service serves. Server reflection is enabled, so fetch it from the running service and generate clients from that, or from the proto revision matching the deployment. Do not assume the vendored ovlive copy matches 0.1.1. Version rules: [versions/api-contracts.md](../versions/api-contracts.md).

Deployment: [deploy/live-edit.md](../deploy/live-edit.md). Operations: [operate/live-edit.md](../operate/live-edit.md).

## Surface

- gRPC only on port `50001`. There is no REST or SSE editing API; port `9090` serves Prometheus `/metrics` only.
- Every LiveService RPC, including `Ping`, needs `authorization: Bearer <token>` metadata.
- gRPC health, server reflection, and `nvidia.omniverse.discovery.capabilities.v2alpha.CapabilitiesService/ListServices` are unauthenticated. The capabilities response is static and is not a readiness check.
- The service name is `LiveService`, not `LiveEditService`.

```protobuf
service LiveService {
  rpc CreateSession(CreateSessionRequest) returns (CreateSessionResponse);
  rpc DeleteSession(DeleteSessionRequest) returns (DeleteSessionResponse);
  rpc ListSessions(ListSessionsRequest) returns (ListSessionsResponse);
  rpc JoinSession(stream JoinSessionRequest) returns (stream JoinSessionResponse);
  rpc CreateLayer(stream CreateLayerRequest) returns (CreateLayerResponse);
  rpc UpdateLayer(stream UpdateLayerRequest) returns (UpdateLayerResponse);
  rpc DeleteLayer(DeleteLayerRequest) returns (DeleteLayerResponse);
  rpc StatLayer(StatLayerRequest) returns (StatLayerResponse);
  rpc SubscribeReadLayer(SubscribeReadLayerRequest) returns (stream SubscribeReadLayerResponse);
  rpc Ping(PingRequest) returns (PingResponse);
}
```

| RPC | Shape | Request → response |
| --- | --- | --- |
| `CreateSession` | unary | `path` (existing base asset address), `name` → `session_id` |
| `ListSessions` | unary | `path` → `sessions[]` (`session_id`, `name`, `created_by_user_name`, `created_at`) |
| `DeleteSession` | unary | `session_id` → empty; deletes its layers |
| `JoinSession` | bidirectional | first `header {session_id, user_application}`, then `message {data}` → first `connected {connection_id}`, then `peer_joined`, `peer_left`, `peer_message` |
| `CreateLayer` | client stream | `header {session_id, layer_name}`, then `chunk {data}` → `object_id` |
| `UpdateLayer` | client stream | `header {session_id, layer_name, object_id, timestamp_microseconds_since_epoch, client_id}`, then chunks → `sequence_num`, `server_timestamp_microseconds_since_epoch` |
| `DeleteLayer` | unary | `session_id`, `layer_name` → empty |
| `StatLayer` | unary | `session_id`, `layer_name` → `created_by_user_name`, `created_at`, `last_modified_at` |
| `SubscribeReadLayer` | server stream | `session_id`, `layer_name`, `object_id`, `sequence_num`, `client_id` → `header` / `chunk` deliveries |
| `Ping` | unary | `client_timestamp_microseconds_since_epoch` → echo plus server timestamp |

Timestamps are microseconds since the Unix epoch. `StatLayer` does not return `object_id` or `sequence_num`; take them from `CreateLayer` and subscription headers. `JoinSession` carries presence and peer messages, not layer content.

### Fields in the vendored proto that 0.1.1 may not serve

ovlive's copy adds, ahead of the service: `CreateLayerHeader.operation_id` and `UpdateLayerHeader.operation_id` (exactly-once re-sends, `OperationOutcome` in responses), `ChainedUpdateLayerHeader` (`UpdateLayerRequest.chained_header`), client keepalive (`JoinSessionHeader.client_keepalive`, `SessionConnected.keepalive_interval_ms`, `JoinSessionKeepalive`), and the `PingResponse.operation_ids` and `.chained_writes` capability flags (`PROVENANCE.md`). Detect support at runtime through those `Ping` flags; an older server ignores unknown fields, so an `OPERATION_OUTCOME_UNSPECIFIED` answer gives no exactly-once guarantee.

## Calling with grpcurl

In protobuf JSON, `uint64` fields are decimal strings and `bytes` are base64.

```bash
LIVE_EDIT=live-edit.example.com:443        # or localhost:50001 via port-forward, adding -plaintext
grpcurl -H "authorization: Bearer ${TOKEN}" \
  -d '{"path":"https://assets.example.com/project/scene.usd","name":"review"}' \
  "$LIVE_EDIT" nvidia.omniverse.live.v1alpha.LiveService/CreateSession
grpcurl -H "authorization: Bearer ${TOKEN}" -d '{"path":"https://assets.example.com/project/scene.usd"}' \
  "$LIVE_EDIT" nvidia.omniverse.live.v1alpha.LiveService/ListSessions
grpcurl -H "authorization: Bearer ${TOKEN}" -d '{"session_id":"<session-id>","layer_name":"root"}' \
  "$LIVE_EDIT" nvidia.omniverse.live.v1alpha.LiveService/StatLayer
```

Layer content cannot be hand-written with grpcurl.

## Layer uploads and generations

- Framing: exactly one header first, then zero or more chunks. A missing or repeated header, or a chunk before the header, is `INVALID_ARGUMENT`.
- Payload: binary OMPF containers holding an OODF delta plus any external values. Not USD text, JSON patches or arbitrary bytes. Use a Live Edit-capable client or its codec (ovlive's `client/source/live/wire/`); do not invent a container format. The vendored proto also requires each external value's 32-byte content hash and size to match its OMPF part.
- Every `CreateLayer` that commits, including an overwrite, allocates a new `object_id` and resets the sequence to 1. `UpdateLayer` must name the current `object_id`; a stale one returns `FAILED_PRECONDITION`. Re-read the current generation before reapplying an edit.

| Limit | Default | Error |
| --- | --- | --- |
| Total upload bytes (`layer.max_size`) | 104857600 | source package: `RESOURCE_EXHAUSTED`; vendored proto comment: `INVALID_ARGUMENT` |
| Single gRPC message | 4 MB (gRPC default) | `OUT_OF_RANGE`; split into chunks |
| Chunk count | `layer.max_size / 1024` | `INVALID_ARGUMENT` |
| Upload capacity full | — | vendored proto: `UNAVAILABLE`, nothing applied, safe to re-send |

The oversize status differs between the two sources; confirm against the deployed service. A connection dropped mid-upload ends with `CANCELLED`.

## Subscriptions and reconnects

Subscribe with `object_id` and `sequence_num` set to the last version fully applied.

| Request | Server behavior |
| --- | --- |
| `0`, `0` | Full content of the current generation |
| Stale or different `object_id` | Full content of the current generation |
| Same generation, behind | One collapsed catch-up diff |
| Already current | Header-only acknowledgement (`total_size: 0`, both timestamps 0), then future updates |

The caught-up acknowledgement is not end-of-stream. Each delivery is a `header {object_id, sequence_num, total_size, ...}` followed by chunks; apply it only after `total_size` bytes arrive, then advance the cursor. Discard partial deliveries on disconnect.

- On `UNAVAILABLE` or a dropped stream, resubscribe with bounded exponential backoff from the last fully applied cursor. gRPC libraries do not resume server streams.
- A stream can end with `OK` (for example during pod shutdown); treat that as reconnect, not done.
- A reconnect may land on another replica; no affinity is needed.
- After a backup restore, reload the layer from `0`, `0` instead of the cached cursor ([operate/live-edit.md](../operate/live-edit.md#checkpoints-and-recovery)).

Self-echo suppression: when `UpdateLayer` and `SubscribeReadLayer` carry the same non-empty `client_id`, the writer does not receive its own updates. Use different IDs when testing delivery between two clients. The source package limits `client_id` to 256 bytes with no control characters or `|` (an empty value disables suppression); the vendored proto adds: use one `client_id` per writer per object.

## Permissions

| RPC | Declared permission (proto comments) | Enforced in 0.1.1 |
| --- | --- | --- |
| `ListSessions`, `JoinSession`, `StatLayer`, `SubscribeReadLayer` | read | Storage read/stat on the base asset |
| `CreateSession`, `DeleteSession`, `CreateLayer`, `UpdateLayer`, `DeleteLayer` | write | Storage read/stat on the base asset |

Access is checked by forwarding the caller's token to Storage; the Permission Service is not called. A caller who can read a base asset can modify its sessions. Long-lived streams check access when they open; revocation does not disconnect them.

## Load balancers

`JoinSession` and `SubscribeReadLayer` are long-lived. Any load balancer or ingress idle timeout ends them; the chart's Contour route allows 240 minutes. Clients must reconnect whenever a stream ends.

## Error codes

| Status | Meaning | Client action |
| --- | --- | --- |
| `UNAUTHENTICATED` | Missing, invalid or expired token | Refresh the token |
| `PERMISSION_DENIED` | Cannot access the base asset | Check access in Storage |
| `NOT_FOUND` | Session, layer or base asset missing | Do not retry with invented IDs |
| `ALREADY_EXISTS` | Session name already used for that asset; or a reused `operation_id` with different content (vendored proto) | Reuse the session; mint a new `operation_id` per intent |
| `FAILED_PRECONDITION` | Stale `object_id` | Re-read the generation |
| `INVALID_ARGUMENT` | Framing, content or field values | Fix the request |
| `RESOURCE_EXHAUSTED` | Upload over `layer.max_size` (source package) | Reduce the upload |
| `OUT_OF_RANGE` | One message over 4 MB | Use smaller chunks |
| `CANCELLED` | Connection dropped mid-upload | Retry |
| `UNAVAILABLE` | Transient service or dependency failure | Bounded backoff |
| `INTERNAL` | Server error (for example Storage lacks `HEAD`) | Report; do not retry indefinitely |

## Clients

- **ovlive** ([GitHub](https://github.com/NVIDIA-Omniverse/ovlive)): C ABI, C++ USD adapter, `omni.usd.live` Python package and a Kit extension over one runtime; `.live` layers and `live://` URIs. Experimental 0.1.0; release targets Kit USD 25.11 and stock USD 25.11 with Python 3.12. Its default `request_timeout_ms` is 35000, above the service's 30-second request timeout. ovlive's `.live` format is separate from Nucleus live layers served by `omni.client`.
- **Client Library** live sessions: see [components/client-library.md](../components/client-library.md). Whether a given Client Library build speaks LiveService 0.1.1 is unverified in this snapshot.
