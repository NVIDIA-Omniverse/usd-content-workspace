# Storage Service in the production stack

> **Applies to:** NVIDIA Storage Service (S3 and Azure Blob Storage adapter) · **Released:** chart and image 1.2.3 (Developer Guide 1.3.0) · **This repo's snapshot:** 1.2.3 + `[Unreleased]` ([`CHANGELOG.md`](../../../../../storage/reference/storage-service/CHANGELOG.md)): URL normalization on by default, permission-client retries, header forwarding to the permission service, Azure path validation, unreleased Storage Core changes (body writes must match `data_object_size`); source commit in [`SOURCE_VERSIONS.md`](../../../../../SOURCE_VERSIONS.md) · **Sources:** the [source skill package](../../../NOTICE.md); [`docs/deployment.rst`](../../../../../storage/reference/storage-service/docs/deployment.rst); [`docs/helm/`](../../../../../storage/reference/storage-service/docs/helm/index.rst); [`helm/values.yaml`](../../../../../storage/reference/storage-service/helm/values.yaml)

The Storage Service serves the Storage API over gRPC (8011) and REST (8012) in front of S3, S3-compatible object stores, and Azure Blob Storage. The authoritative configuration reference is the chart documentation in [`docs/helm/`](../../../../../storage/reference/storage-service/docs/helm/index.rst); the in-repo skill [`skills/storage-service/SKILL.md`](../../../../../storage/reference/storage-service/skills/storage-service/SKILL.md) maps tasks to pages, and [../../components/storage-service.md](../../components/storage-service.md) adds what it lacks. The 1.2.3 release documentation is published at `https://docs.omniverse.nvidia.com/ovstorage/storage-service/1.2.3/` (for example [`helm/storage-backend.html`](https://docs.omniverse.nvidia.com/ovstorage/storage-service/1.2.3/helm/storage-backend.html)).

**Released versus snapshot.** For a 1.2.3 deployment, use the 1.2.3 chart and its published docs. This repository's docs describe 1.2.3 plus unreleased changes; the most consequential is `config.urlNormalizer.enabled` defaulting to `true` ([`url-normalizer.rst`](../../../../../storage/reference/storage-service/docs/helm/url-normalizer.rst)), which changes which object some addresses resolve to. Audit keys with empty path segments (`a//b`) or `.`/`..` segments before running a snapshot build against existing data, or set `config.urlNormalizer.enabled: false`.

The snapshot also builds on unreleased Storage Core changes (by its `Cargo.lock`): a body write (`upload_preference=body` or gRPC chunks) now reads exactly `data_object_size` bytes, rejecting a shorter body with `INVALID_ARGUMENT`/400 and silently truncating a longer one. Clients that send a wrong size against 1.2.3 without noticing will see failures or truncated objects against a snapshot build. No chart value controls this.

Security note (true of the previous and current snapshots; the released 1.2.3 image was not inspected): the permission layer does not authorize the v1alpha `IsReadOnly` and `IsVersioned` capability queries; any caller the ingress lets through gets an answer for any address ([`backend/permissions/`](../../../../../storage/reference/storage-service/storage-service/src/backend/permissions/)). With `config.manifest.enabled`, Storage Core's manifest README states that a warm manifest cache answers these queries for workspace paths without a permission check on the `.manifest`, so the answer can reveal whether a path is listed in a workspace manifest the caller cannot read.

## Inputs to settle first

- Backend type and the exact endpoint addresses (bucket or storage-account URLs).
- Backend identity: workload identity (EKS IRSA, AKS Workload Identity), service-identity token file, credentials file, static keys, or anonymous read-only.
- Metadata backend, if the Metadata API is needed (PostgreSQL or CosmosDB; [`storage-metadata.rst`](../../../../../storage/reference/storage-service/docs/helm/storage-metadata.rst)).
- Whether permissions, notifications, ingress, and auth are in scope now.

Leave anything unknown unresolved in the values file and say so; do not invent identities or endpoints.

## Backend permissions

**AWS S3** — minimal policy on the bucket or the role ([`deployment.rst`](../../../../../storage/reference/storage-service/docs/deployment.rst) "Required permissions for AWS S3 buckets"):

```json
{
  "Version": "2012-10-17",
  "Statement": [
    { "Sid": "BucketLevelPermissions", "Effect": "Allow",
      "Action": ["s3:ListBucket", "s3:ListBucketVersions"],
      "Resource": "arn:aws:s3:::<bucket-name>" },
    { "Sid": "ObjectLevelPermissions", "Effect": "Allow",
      "Action": ["s3:GetObject", "s3:GetObjectVersion", "s3:PutObject", "s3:DeleteObject",
                 "s3:DeleteObjectVersion", "s3:AbortMultipartUpload", "s3:ListMultipartUploadParts"],
      "Resource": "arn:aws:s3:::<bucket-name>/*" }
  ]
}
```

The version actions are only needed for versioned buckets.

**Azure Blob Storage** — assign **Storage Blob Data Contributor** at the storage-account scope, or a custom role. The snapshot's minimal custom role adds two control-plane actions and one data action that the source skill package role omitted: `Microsoft.Storage/storageAccounts/blobServices/containers/read`, `.../blobServices/generateUserDelegationKey/action` (needed to sign user-delegation SAS URLs with Entra credentials), and `.../containers/blobs/add/action`. Use the role in [`deployment.rst`](../../../../../storage/reference/storage-service/docs/deployment.rst) "Required permissions for Azure Blob storage accounts".

**CORS** — only if browsers (Navigator) reach the backend directly through redirect URLs. Allow your app origins, methods `GET PUT POST DELETE HEAD`, headers `*`, expose `ETag` and `Content-Length`. For S3 use `aws s3api put-bucket-cors --bucket <bucket> --cors-configuration file://cors.json`; for Azure the snapshot docs use `az storage account blob-service-properties cors-rule add` (the source skill package used `az storage cors add --services b`).

## Get the chart

- Released: `storage-service-1.2.3.tgz` from NGC ([../artifact-access.md](../artifact-access.md)).
- Snapshot: [`storage/reference/storage-service/helm/`](../../../../../storage/reference/storage-service/helm/). Its `Chart.yaml` is `0.0.0` and `image.tag` defaults to `"0.0.0"` (the image helper is `repository:tag` with no `appVersion` fallback), so always set `image.tag`.

## Values

Base:

```yaml
image:
  repository: nvcr.io/nvidia/omniverse/storage-service
  tag: "1.2.3"                      # the image you deploy
  pullSecrets:
    - name: ngcpull-secret
```

The snapshot chart already runs as UID/GID 1000 non-root, drops all capabilities, and forbids privilege escalation (1.2.3 changelog). Keep those defaults.

### S3

Endpoint keys are **addresses**, `<scheme>://<host>(/<path>)?`, not bare hostnames. A bucket in the path (`https://s3.us-west-1.amazonaws.com/mybucket`) selects path-style addressing. `allowUnconfiguredEndpoints` defaults to `false`, so requests to undeclared buckets are rejected.

Workload identity on EKS (preferred):

```yaml
serviceAccount:
  create: true
  annotations:
    eks.amazonaws.com/role-arn: "arn:aws:iam::<account-id>:role/<role-name>"
config:
  storage:
    s3:
      endpoints:
        "https://<bucket>.s3.<region>.amazonaws.com":
          credentials:
            webIdentityToken:
              webIdentityTokenFile: "${AWS_WEB_IDENTITY_TOKEN_FILE}"   # injected by the EKS webhook
              roleArn: "${AWS_ROLE_ARN}"
```

`${VAR}` values are interpolated inside the pod ([`env-var-interpolation.rst`](../../../../../storage/reference/storage-service/docs/helm/env-var-interpolation.rst)); they are not literal values. Static keys, only if chosen:

```yaml
config:
  storage:
    s3:
      endpoints:
        "https://<bucket>.s3.<region>.amazonaws.com":
          credentials:
            accessKey:
              accessKeyId: "${STORAGE_SERVICE_AWS_ACCESS_KEY_ID_1}"
              secretAccessKey: "${STORAGE_SERVICE_AWS_SECRET_ACCESS_KEY_1}"
extraEnvFrom:
  - secretRef:
      name: storage-service-secrets-aws   # created out of band; never commit its contents
```

Other providers: `credentials.anonymous: true` (read-only public buckets), `credentials.credentialsFile.path` (file written by a sidecar, renewed at least 60 s before `expires_on`), and `credentials.assumeRole` layered on a static, web-identity, or credentials-file base (not chainable). For S3 endpoints `credentials` is marked required in [`storage-backend.rst`](../../../../../storage/reference/storage-service/docs/helm/storage-backend.rst); there is no implicit SDK default chain, so pick a provider. The flat pre-1.2.0 keys (`accessKeyId`, `assumedRoleArn`, ...) are gone ([`migration.rst`](../../../../../storage/reference/storage-service/docs/helm/migration.rst)).

S3-compatible stores (MinIO, Dell ECS, Ceph RGW, ...) use the same `endpoints` map plus `bucket`, `region`, `endpointUrl`, and `addressingStyle` (`path` or `virtual`) where they cannot be inferred ([`deployment.rst`](../../../../../storage/reference/storage-service/docs/deployment.rst) "Other S3 API providers", [`storage-backend.rst`](../../../../../storage/reference/storage-service/docs/helm/storage-backend.rst)). Before calling a store compatible, test list/stat, conditional writes, versions, copy/move, multipart transfers, and client downloads through presigned URLs. Treat ETags as opaque.

The source skill package reported that the release-pinned chart's endpoint-normalizing helper could prepend `https://` twice to a complete URL. Inspect the endpoint section of `helm template` output for the chart you install.

### Azure Blob Storage

Endpoints live under `config.storage.azureBlob.endpoints` (not `config.storage.azure.azureBlob`, as the source skill package shows). Workload identity (preferred):

```yaml
serviceAccount:
  create: true
  annotations:
    azure.workload.identity/client-id: "<client-id>"
    azure.workload.identity/tenant-id: "<tenant-id>"
podLabels:
  azure.workload.identity/use: "true"
config:
  storage:
    azureBlob:
      endpoints:
        "https://<account>.blob.core.windows.net":
          credentials:
            federated:
              clientId: "${AZURE_CLIENT_ID}"
              tenantId: "${AZURE_TENANT_ID}"
              federatedTokenFile: "${AZURE_FEDERATED_TOKEN_FILE}"
              authorityHost: "${AZURE_AUTHORITY_HOST}"
```

Account key, only if chosen: `credentials.storageKey.storageKey: "${STORAGE_SERVICE_AZURE_STORAGE_KEY_1}"` with the variable from a Secret via `extraEnvFrom`. The source skill package shows `credentials.storageAccount.{storageAccount,storageKey}`; the snapshot docs use `storageKey.storageKey` and infer the account from the URL. Check the released chart's `storage-backend` page before choosing. An endpoint with no `credentials` uses the access-token-file service identity when `serviceIdentity.accessTokenFile.enabled` is true ([`storage-credentials.rst`](../../../../../storage/reference/storage-service/docs/helm/storage-credentials.rst)).

### Per-endpoint switches

`readOnly`, `versioningEnabled`, and `permissionsEnabled` (renamed from `enablePermissions` in 1.2.1; the old key is silently ignored) exist on both S3 and Azure endpoints in the snapshot ([`storage-backend.rst`](../../../../../storage/reference/storage-service/docs/helm/storage-backend.rst)).

### Hooks for other layers

| Layer | Storage Service keys | Configure with |
| --- | --- | --- |
| Authorization | `config.permissions.enabled`, `config.permissions.client.grpc.endpoint` (default `http://permission-service:3010`) or `client.rest` | [../permissions-and-auth.md](../permissions-and-auth.md), [`storage-permissions.rst`](../../../../../storage/reference/storage-service/docs/helm/storage-permissions.rst) |
| Service identity (outbound calls) | `serviceIdentity.accessTokenFile.*` | [`storage-credentials.rst`](../../../../../storage/reference/storage-service/docs/helm/storage-credentials.rst) |
| Notifications | `config.storageEvents.<subscriber-id>.{sqs,azureServiceBus}`, `config.notificationClient.enabled`, `config.notificationClient.endpointUrl` (default `http://event-aggregation-service:50051`), `config.syntheticDirEvents.enabled` (default `true`) | [../notifications.md](../notifications.md), [`storage-backend-events.rst`](../../../../../storage/reference/storage-service/docs/helm/storage-backend-events.rst) |
| Metadata API | `config.metadata.<backend-id>.{postgres,...}` and endpoint binding | [`storage-metadata.rst`](../../../../../storage/reference/storage-service/docs/helm/storage-metadata.rst), [../../apis/metadata.md](../../apis/metadata.md) |
| Caching, L2 Redis/Valkey | `config.smallObjectCache`, `statCache`, `listCache`, `metadataCache`, `config.redis.*` | [`storage-backend-caching.rst`](../../../../../storage/reference/storage-service/docs/helm/storage-backend-caching.rst), [../../operate/scalability.md](../../operate/scalability.md) |
| Observability | `observability.metrics/tracing/logging`, `service.metrics.enabled` | [`observability.rst`](../../../../../storage/reference/storage-service/docs/helm/observability.rst), [../../operate/monitoring.md](../../operate/monitoring.md) |
| Ingress | `httpProxy.*` | [ingress-and-tls.md](ingress-and-tls.md) |

The storage events backend credentials (SQS, Service Bus) do not authenticate notification publishing; publishing uses the service identity.

## Register with Discovery

The chart adds no discovery annotations by default. Add them through `service.annotations`:

```yaml
service:
  annotations:
    discovery.omniverse.nvidia.com/enabled: "true"
    discovery.omniverse.nvidia.com/visibility: "both"     # or internal / external
```

The Service's ports are named `grpc` (8011) and `rest` (8012), which is what the operator resolves; no `grpc-port` annotation is needed. For external URLs either enable `httpProxy` (the operator maps the HTTPProxy routes to `storage.<domain>` and `grpc.storage.<domain>`) or set `discovery.omniverse.nvidia.com/external-rest-endpoint` / `external-grpc-endpoint` to client-reachable URLs.

In the snapshot chart, `service.annotations` is rendered on both `storage-service` and `storage-service-headless` ([`service-headless.yaml`](../../../../../storage/reference/storage-service/helm/templates/storage-service/service-headless.yaml)), and the operator has no headless filter, so expect a second catalog entry for the headless Service. Clients that select by capability and a configured service name are unaffected; check the catalog. Details: [../discovery.md](../discovery.md).

## Install and validate

```bash
helm lint <chart> -f storage-values.yaml
helm template storage-service <chart> -f storage-values.yaml -n <ns> > /dev/null   # rendered output can contain secrets
helm upgrade --install storage-service <chart> -f storage-values.yaml -n <ns>
kubectl -n <ns> rollout status deployment/storage-service
```

From inside the cluster (or after `kubectl -n <ns> port-forward service/storage-service 8012:8012`):

```bash
curl http://storage-service.<ns>.svc.cluster.local:8012/v1alpha/capabilities/services   # storage capabilities (also /v1beta/...)
curl http://storage-service.<ns>.svc.cluster.local:8012/v2alpha/services                # discovery capabilities, REST form
curl "http://storage-service.<ns>.svc.cluster.local:8012/v1alpha/filefolder/list/https%3A%2F%2F<bucket>.s3.<region>.amazonaws.com%2F"
```

Use a bucket you declared; an undeclared bucket is rejected while `allowUnconfiguredEndpoints` is `false`. A write/read round trip with the intended identity proves backend access; a 200 from capabilities does not. More: [validation.md](validation.md).

Upgrades: read the installed-to-target `CHANGELOG.md` entries and [`migration.rst`](../../../../../storage/reference/storage-service/docs/helm/migration.rst); see [../../operate/upgrades.md](../../operate/upgrades.md).
