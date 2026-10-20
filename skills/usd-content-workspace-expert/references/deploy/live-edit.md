# Deploy the Live Edit Service

> **Applies to:** Live Edit Service (external to this repository) · **Released:** chart and image `live-edit-service` 0.1.1 (Developer Guide 1.3.0 / NGC inventory; absent from the previous `published` profile) · **This repo's snapshot:** not present. The service source and chart are not in this repository; they are in [ovlive](https://github.com/NVIDIA-Omniverse/ovlive) under `service/`, beside the client runtime under `client/` (client 0.1.0 experimental) · **Sources:** the [source skill package](../../NOTICE.md) (its summary of the released 0.1.1 chart and guide), [usd-content-workspace Developer Guide](https://docs.omniverse.nvidia.com/ovstorage/ovstorage-guide/latest/index.html); Discovery registration checked against this repo's [operator README](../../../../discovery/reference/discovery-service/operator/README.md)

Every chart key, default and behavior on this page comes from the source package's description of the released 0.1.1 chart. None can be checked against source in this repository: treat them as **unverified in this snapshot**, and read the extracted 0.1.1 chart's `values.yaml` before generating values. Confirm what the caller actually runs with [capture-a-deployment](../versions/capture-a-deployment.md).

Related: [apis/live-edit.md](../apis/live-edit.md) (RPCs, framing, reconnects), [operate/live-edit.md](../operate/live-edit.md) (validation, recovery, metrics), [components/live-edit.md](../components/live-edit.md).

## What Live Edit is

A stateful gRPC service (`nvidia.omniverse.live.v1alpha.LiveService`) that holds collaborative scene-layer sessions on top of an Omniverse Storage APIs deployment. It is optional and independent of Notifications: it uses Valkey (or Redis 7.x), not RabbitMQ. Installing it does not make an arbitrary USD application collaborative; clients need Live Edit protocol support (for example ovlive). Live sessions never save into the base asset; saving is the application's job.

| Component | Purpose | Provided by |
| --- | --- | --- |
| Live Edit Service | Sessions, presence, layer edits and subscriptions | `live-edit-service` chart 0.1.1 |
| Valkey 7.2+ or Redis 7.x | Authoritative live state | You (self-hosted or managed); not in the chart, and NVIDIA ships no Valkey chart |
| Storage Service | Base assets; access check for every Live Edit request | Your existing stack ([deploy/production-stack.md](production-stack.md)) |
| Checkpoint store (optional) | Backups of live state for recovery after Valkey loss | A Storage API location you choose |

| Port | Protocol | Purpose |
| --- | --- | --- |
| `50001` | gRPC (HTTP/2, plaintext in cluster) | LiveService, gRPC health, reflection, Discovery capabilities |
| `9090` | HTTP | Prometheus `/metrics` only |

## Prerequisites

- Kubernetes with Helm, `kubectl`, and gRPC probe support; NGC access to the 0.1.1 chart and image ([deploy/artifact-access.md](artifact-access.md)).
- An authenticated Storage Service holding the base assets. The default REST access check calls `HEAD /v1alpha/fileobject/by-address/{address}` with the caller's token, so the endpoint must support `HEAD` and validate tokens.
- Client tokens that Storage accepts. With User Info disabled, tokens must be JWTs carrying a non-empty `name`, `preferred_username`, `sub` or `oid` claim.
- Valkey meeting the requirements below, one keyspace per Live Edit deployment.
- For external checkpoints: a dedicated backup prefix in Storage, a service identity with read, list, write and delete there, and a renewable token provider.

The Storage endpoint, a base asset address and the checkpoint prefix are three different settings, for example endpoint `https://storage.example.com`, asset `https://assets.example.com/project/scene.usd`, backup prefix `https://assets.example.com/live-edit-backups/`.

## Valkey

Valkey holds authoritative live state, not a cache.

| Requirement | Setting |
| --- | --- |
| Eviction | `maxmemory-policy noeviction`; any evicting policy corrupts layer data |
| Memory | Explicit `maxmemory` below the pod or host limit |
| Durability | AOF (`appendonly yes`, `appendfsync everysec`) and/or Live Edit checkpoints |
| ACL | Data commands, Lua scripting, transactions, streams, pub/sub; a read-only user is not enough |
| Network | Pods must reach every node and port a clustered store advertises |
| Production | TLS, authentication, replication and failover |

Self-hosted minimum, on persistent volumes:

```text
maxmemory <below the container memory limit>
maxmemory-policy noeviction
appendonly yes
appendfsync everysec
```

Managed stores may hide `CONFIG GET appendonly`; Live Edit then starts with a warning and does not verify durability, so confirm the provider's persistence yourself.

| Topology | URL scheme |
| --- | --- |
| Single primary, TLS | `rediss://` |
| Cluster-aware endpoint, TLS (MemoryDB, Azure Managed Redis, Valkey cluster mode) | `rediss-cluster://` |
| Development only, no TLS | `redis://`, `redis-cluster://` |

A URL that embeds credentials belongs in a Secret, never in values. The chart reads key `valkey-url` from the Secret named by `envSecretName` (default `live-edit-env`) unless `valkey.url.value` or `valkey.url.valueFrom` is set (set at most one). URL-encode reserved characters in embedded credentials.

```bash
# Read the URL from a protected file or your secret manager; do not paste it into shell history.
kubectl -n <namespace> create secret generic live-edit-env --from-file=valkey-url=./valkey-url.txt
```

```yaml
valkey:
  url:
    value: "rediss-cluster://valkey.example.com:6379"   # credential-free URL only
    # or: valueFrom: { secretKeyRef: { name: live-edit-valkey, key: url } }
```

## Values

### Choose durability first

The chart defaults to `checkpoint.enabled: true` with `checkpoint.adapter: Disabled`, which the service refuses at startup.

| Mode | Values | Protects against |
| --- | --- | --- |
| Backing-store durability only | `checkpoint.enabled: false` | Pod and Valkey restarts with AOF. Losing the whole Valkey dataset loses all sessions. Startup is refused if Valkey reports `appendonly no`. |
| External checkpoints (production) | `checkpoint.enabled: true`, `checkpoint.adapter: StorageApi` | Also recovers after complete Valkey loss |
| Local disk (single pod, development) | `checkpoint.adapter: LocalDisk`, `localDiskPath` on a persistent mount, `replicaCount: 1`, `autoscaling.enabled: false` | Not shared; never for multiple replicas |

### Base values

```yaml
image:
  repository: nvcr.io/nvidia/omniverse/live-edit-service
  tag: "0.1.1"
  pullSecrets:
    - name: ngcpull-secret
envSecretName: live-edit-env
authorization:
  enabled: true
  protocol: rest                    # or grpc with the Storage gRPC endpoint
  storageServiceEndpoint: "http://storage-service.<namespace>.svc.cluster.local:8012"
userInfo:
  enabled: false
checkpoint:
  enabled: false
```

Use an HTTPS Storage endpoint outside a trusted cluster network. Chart defaults: 3 replicas, HPA 3–10, PodDisruptionBudget `minAvailable: 2`.

### External checkpoints

```yaml
checkpoint:
  enabled: true
  adapter: StorageApi
  storageApiEndpoint: "https://storage.example.com"
  basePath: "https://assets.example.com/live-edit-backups/"
  interval: 60                      # seconds; the key is `interval`
  credentials:
    accessTokenFile:
      path: /etc/secrets/access-token/access_token
      pollIntervalSecs: 15
serviceIdentity:
  accessTokenFile:
    enabled: true
    provisionerSidecar:
      container:
        image: "<token-provisioner-image>:<tag>"
```

- `basePath` is a registered storage address, not the API endpoint; backups go under `<basePath>/.live-edit/`. Dedicate the prefix to one deployment.
- The provider writes `{"access_token": "...", "expires_on": "<RFC 3339>"}` atomically and refreshes before expiry.
- Keep three credentials separate: the outbound checkpoint token (Storage audience), callers' inbound tokens (forwarded for access checks), and Valkey credentials.
- `serviceIdentity.clientCredentials` does not acquire tokens in 0.1.1; use the token file.
- The interval is a schedule, not a recovery-point guarantee.
- Large backups upload directly to a presigned object-store URL returned by Storage. Allow DNS, TLS trust and HTTPS egress to the bucket host. Never forward the Storage token to that host, and never log presigned URLs.

## Authentication and authorization

| Surface | Configuration | Notes |
| --- | --- | --- |
| Caller access checks | `authorization.enabled: true`, `protocol: rest` or `grpc`, `storageServiceEndpoint` | Forwards the caller's token to Storage (`HEAD` by address, or gRPC `FileObjectService.Stat`). Does not call the Permission Service. |
| Caller identity | `userInfo.enabled: false` decodes JWT display claims | No signature, issuer, audience or expiry validation here; Storage is the validation boundary. Opaque tokens need `userInfo.enabled: true` and `userInfo.endpoint` ([deploy/userinfo.md](userinfo.md)). |
| Checkpoint identity | `serviceIdentity.accessTokenFile` + `checkpoint.credentials.accessTokenFile.path` | Enabling it also enables a service-token readiness check, even with checkpoints off |
| Valkey | URL credentials, `valkey.credentials.accessTokenFile`, or `valkey.credentials.awsIam` | At most one mode |

Authentication cannot be disabled; disabling both authorization and User Info is rejected. A `405` from Storage on access checks (clients see `INTERNAL`) means the endpoint lacks `HEAD`; fix the endpoint, do not disable authorization. `protocol: grpc` also switches User Info calls to gRPC.

**Authorization boundary (0.1.1):** read and modifying RPCs all use Storage's read/stat check. Anyone who can read a base asset can modify its live sessions and layers. There is no read-only participant role. Restrict base-asset read access to intended editors ([operate/known-issues.md](../operate/known-issues.md)). The vendored proto comments describe write permission for modifying RPCs; the released service does not enforce that distinction.

### Azure: passwordless Valkey

```yaml
valkey:
  url:
    value: "rediss-cluster://<cache>.<region>.redis.azure.net:10000"
  credentials:
    accessTokenFile:
      enabled: true
      username: "<backing-store-principal-id>"
      path: /etc/secrets/access-token/valkey_access_token
```

Requires `serviceIdentity.accessTokenFile.enabled: true`. Storage and Redis tokens are separate files with separate audiences in `/etc/secrets/access-token`. Refresh each at least 180 seconds plus the poll interval before expiry (195 s with the 15 s default), with margin. The provider must also keep the Storage-audience token at `checkpoint.credentials.accessTokenFile.path` or pods stay unready.

### AWS: IAM (MemoryDB, ElastiCache)

```yaml
valkey:
  url:
    value: "rediss-cluster://clustercfg.<cluster>.<id>.memorydb.<region>.amazonaws.com:6379"
  credentials:
    awsIam:
      enabled: true
      service: memorydb             # or elasticache
      username: "<acl-user>"
      region: "<region>"
      clusterHost: "<cluster-name>" # name used for signing, not the endpoint FQDN
serviceAccount:
  annotations:
    eks.amazonaws.com/role-arn: "arn:aws:iam::<account>:role/live-edit-valkey"
```

Create the IAM-enabled ACL user and grant the role (for example `memorydb:connect`) first.

## Ingress

```yaml
httpProxy:
  enabled: true
  fqdn:
    host: live-edit               # chart default host is grpc.live-edit
    domain: example.com
  tls:
    enabled: true
    secretName: live-edit-tls
```

Requires Contour; routes to `50001` over h2c. `JoinSession` and `SubscribeReadLayer` get 240-minute response and idle timeouts; other RPCs use `httpProxy.timeoutPolicy` (30-second defaults), so raise it for large uploads. No session affinity is needed. The built-in `networkPolicy` is off by default and does not cover every dependency; before enabling it, allow DNS, every Valkey node, Storage, User Info, token providers, object-store upload hosts and telemetry. To put Live Edit behind the Envoy auth extension, see [deploy/permissions-and-auth.md](permissions-and-auth.md).

## Install

```bash
helm fetch https://helm.ngc.nvidia.com/nvidia/omniverse/charts/live-edit-service-0.1.1.tgz \
  --username='$oauthtoken' --password="${NGC_API_KEY}"
tar -xzf live-edit-service-0.1.1.tgz
helm lint ./live-edit-service -f live-edit-values.yaml
helm template live-edit-service ./live-edit-service -n <namespace> -f live-edit-values.yaml > rendered.yaml
helm upgrade --install live-edit-service ./live-edit-service -n <namespace> -f live-edit-values.yaml
kubectl -n <namespace> rollout status deployment/live-edit-service --timeout=300s
```

The startup probe allows 280 seconds; keep the chart's gRPC probes.

### Optional smoke tests

With `tests.enabled: true` the chart's tests create and delete a disposable Storage fixture and a session; they do not edit layer content. They also run as post-install and post-upgrade hooks, so a failing test fails `helm upgrade --install`. They always use the Storage REST endpoint from `authorization.storageServiceEndpoint`.

```yaml
smokeTest:
  image:
    pullSecrets:
      - name: ngcpull-secret
tests:
  enabled: true
  storageApiResourceBase: "https://assets.example.com/live-edit-tests/"
  auth:
    microsoftUrl:
      value: "https://login.microsoftonline.com/<tenant>/v2.0/.well-known/openid-configuration"
    credentials:
      valueFrom:
        secretKeyRef: { name: live-edit-smoke-auth, key: credentials }   # CLIENT_ID:CLIENT_SECRET, existing Secret
    scope:
      value: "<storage-api-audience>/.default"
```

```bash
helm test live-edit-service -n <namespace> --logs
```

## Register with Discovery

Add to the Live Edit Service:

```yaml
service:
  annotations:
    discovery.omniverse.nvidia.com/enabled: "true"
    discovery.omniverse.nvidia.com/grpc-port: "50001"
```

The operator calls the unauthenticated `nvidia.omniverse.discovery.capabilities.v2alpha.CapabilitiesService/ListServices`, which advertises `LiveService`. Live Edit has no REST endpoint. This repo's operator snapshot also maps `nvidia.omniverse.live.v1alpha.LiveService` to v1 types `live-edit` and `live-service` for older clients (`v1Endpoints` in the operator values). Use the visibility and external-gRPC-endpoint annotations when the advertised route must differ ([deploy/discovery.md](discovery.md)). Validate `/api/v2/services` with `"schema-version": 2`, then test from the client network; a registry entry does not prove readiness, authorization or edit delivery ([operate/live-edit.md](../operate/live-edit.md)).

## Secrets

| Secret | Keys | Used by |
| --- | --- | --- |
| `ngcpull-secret` | registry credentials for `nvcr.io` | image pull |
| `live-edit-env` | `valkey-url` | Valkey URL (not needed when `valkey.url` is set) |
| `live-edit-smoke-auth` | `credentials` | chart smoke tests |
| `live-edit-tls` | certificate and key | Contour HTTPProxy |
