# usd-content-workspace integration tests (released chart)

> **Applies to:** `storage-api-integration-tests` Helm chart and `integration-tests` image · **Released:** chart and image **1.0.23** (usd-content-workspace Developer Guide 1.3.0 / NGC inventory); previous `published` profile **1.0.18** · **This repo's snapshot:** **not in this repository** — no chart, image source, values or markers to verify against · **Sources:** the [source skill package](../../NOTICE.md) (its inspection of the released 1.0.23 chart and of the 1.0.18 image), [usd-content-workspace NGC collection](https://catalog.ngc.nvidia.com/orgs/nvidia/teams/omniverse/collections/storage_apis); cross-checks against this repo's Discovery operator and smoke-test base image notes

Everything below comes from the source skill package, which inspected the released chart. None of it can be checked against files here. Before applying any value, fetch the chart version you will run, read its `values.yaml` and templates, and confirm the markers from the image's `pytest.ini` or a first run's collection output. Confirm what the target stack runs first ([capture a deployment](../versions/capture-a-deployment.md)).

## What it tests

A Kubernetes Job that exercises deployed services together, over gRPC and REST:

- Storage + Permissions: enforcement on allow and deny paths.
- Storage + Notifications: storage writes produce events.
- Notifications on their own: aggregation and consumer.
- Storage + Thumbnail and Live Edit (1.0.23).

A preflight validates required values and endpoint reachability. Tests for services that are neither discovered nor configured are **deselected**, so a non-zero deselected count is normal. The signal that endpoints are right is the "Discovered Services" line, not the count:

```text
Discovered Services: Storage (gRPC,REST), Permission (gRPC,REST),
                     Event Consumer (gRPC,REST), Event Aggregation (gRPC,REST)
Deselected 13 tests due to unavailable services
TEST SUMMARY  Total tests run: 44   Passed: 44   OVERALL STATUS: PASSED
```

Record exit status and selected, executed, passed, failed, skipped and deselected counts. A completed Job or an empty selection is not a pass. For notifications, also prove the client path yourself: subscribe first, write a disposable object, and correlate the received event.

It complements, not replaces, the per-service checks in [conformance.md](conformance.md).

## Chart coordinates

| Field | Value (1.0.23) |
|---|---|
| Chart | `storage-api-integration-tests` |
| Chart URL | `https://helm.ngc.nvidia.com/nvidia/omniverse/charts/storage-api-integration-tests-1.0.23.tgz` |
| Image | `nvcr.io/nvidia/omniverse/integration-tests`, tag defaults to chart `appVersion` |

```bash
# NGC_API_KEY comes from the operator's own environment or credential store; never paste it into chat or commit it.
helm fetch https://helm.ngc.nvidia.com/nvidia/omniverse/charts/storage-api-integration-tests-1.0.23.tgz \
  --username='$oauthtoken' --password="${NGC_API_KEY}"
tar -xzf storage-api-integration-tests-1.0.23.tgz && cd storage-api-integration-tests
```

Mirrors and private registries: override `image.repository` and `image.pullSecrets`. Registry access, chart download, image pull and a passing run are four separate checks ([deploy/artifact-access.md](../deploy/artifact-access.md)).

## Values

Image fields are top level (`image.repository`, `image.tag`, `image.pullPolicy`, `image.pullSecrets`); there is no `job.image`.

### Required

```yaml
image:
  repository: nvcr.io/nvidia/omniverse/integration-tests
  pullSecrets: [{ name: <pull-secret> }]
job:
  storageApiResourceBase: "https://<bucket-or-container-url>/<disposable-test-prefix>/"
  discoveryServiceEndpoint: "http://<discovery-service>:<port>"
```

**The suite writes real data** under `job.storageApiResourceBase` (for example `<base>/test/test-<id>/file-<id>`) and does not remove all of it. Use a dedicated prefix, never production content.

### Endpoints: pick one mode

- **Discovery:** `job.discoveryServiceEndpoint` is the base URL; the chart appends `/api/v1/services`. Do not include that path. The snapshot Auto Discovery operator in this repo serves `/api/v1/services` as a v1-compatibility view beside `/api/v2/services` ([`src/api/rest.rs`](../../../../discovery/reference/discovery-service/operator/src/api/rest.rs)); confirm the released operator you run does the same.
- **Manual:** leave `discoveryServiceEndpoint` empty and set `job.manualEndpoints.{grpcStorage,httpStorage,grpcPermission,httpPermission,grpcNotificationConsumer,httpNotificationConsumer,grpcNotificationAggregation,httpNotificationAggregation}`. Each set field becomes a `*_ENDPOINT` env var; unset services are deselected.

Use the ports of the charts you actually deployed (for example Storage Service gRPC `8011`, REST `8012` in its snapshot chart); the source skill's example used `:8011`/`:8012` for every service, which matches neither notification chart (both default gRPC `50051` and REST `8000` in this snapshot).

### Authentication: exactly one mode

| Mode | Values |
|---|---|
| None | both blocks disabled; only for stacks without the Envoy auth extension |
| OAuth2 client credentials | `serviceIdentity.clientCredentials.{enabled,openIdConfigurationUri,clientScope,clientId,clientSecretRef.{name,key}}`; the secret comes only from an existing Kubernetes Secret |
| Workload identity token file | `serviceIdentity.accessTokenFile.{enabled,path,provisionerSidecar.container}`, plus `podLabels."azure.workload.identity/use": "true"` and `serviceAccount.annotations."azure.workload.identity/client-id"` |

Token-file mode notes: needs Kubernetes **1.29+** (the sidecar is a native sidecar init container); `path` must equal the sidecar's `TOKEN_OUTPUT_PATH` under `/etc/secrets/access-token`; rendering fails without `provisionerSidecar.container.image`. The federated credential binds to `system:serviceaccount:<namespace>:<serviceaccount>`, so changing namespace or ServiceAccount name breaks it (`AADSTS70021`/`AADSTS70025`). `serviceAccount.name` defaults to `job.name`; with `serviceAccount.create: false` you must set `serviceAccount.name`, and the existing SA must already carry the client-id annotation.

### Markers

`job.pytestMarkers` is a pytest marker expression; empty runs everything.

| Marker (1.0.23) | Runs |
|---|---|
| `rest`, `grpc` | One transport |
| `storage_and_permission` | Storage + Permission |
| `storage_and_notification` | Storage + Notification |
| `storage_and_thumbnail` | Storage + Thumbnail |
| `live_edit` | Live Edit (gRPC) |
| `permit` | Allow paths using existing permits |
| `forbid` | Policy-mutating denial checks; deselected when `setupPolicies: false` |

There is no bare `storage` or `notification` marker; an unknown marker selects zero tests. Selecting one interaction says nothing about the others.

### Policies and options

| Key | Default | Notes |
|---|---|---|
| `job.setupPolicies` | `true` | Tests create and delete their own permits; the test principal needs `permissions:edit`. `false`: pre-provision Storage `read`/`write` on the test prefix; for notification tests Consumer `create-durable-queues`, `consume-durable-queues`, `delete-durable-queues`, `consume-all-storage-create-events`, `consume-all-storage-delete-events` and Aggregation `publish-event` for `omni.storage.created`. `forbid` cases are auto-deselected |
| `job.timeoutSeconds` | `10` | Per request |
| `job.skipConnectivityChecks` | `false` | Skip preflight probes |
| `job.pytestTechnicalOutput` / `pytestTraceback` / `pytestLogLevel` | `"false"` / `"no"` / `"INFO"` | Use `"true"` + `"short"` to debug |
| `job.name` | `storage-api-integration-tests` | Not derived from the release name; see below |
| `job.restartPolicy`, `backoffLimit`, `ttlSecondsAfterFinished` | `Never`, `0`, `""` | |
| `job.securityContext` | `runAsNonRoot: true`, `runAsUser: 1000`, `allowPrivilegeEscalation: false` | Keep `runAsUser: 1000`: the image sets `USER nvs` by name, so without a numeric UID the pod fails `CreateContainerConfigError: image has non-numeric user (nvs)` |

Prefer the least-privilege `setupPolicies: false` mode where the test principal should not edit policies; expect fewer executed tests.

## Install, run, capture

The Job is a Helm hook (`post-install,post-upgrade`, delete policy `before-hook-creation,hook-succeeded`):

1. It runs on every `helm upgrade` of the release.
2. `helm upgrade --install` blocks until the Job finishes.
3. A **successful** Job is deleted immediately; failed Jobs remain. `ttlSecondsAfterFinished` cannot keep a succeeded hook.
4. `job.name` is shared by default, so a second release in the same namespace **deletes the first release's Job**. Set a unique `job.name` (and pin `serviceAccount.name` if workload identity is bound to the old name).

Check first, then start a log watcher before installing:

```bash
helm list -n <namespace> | grep storage-api-integration-tests; kubectl get jobs -n <namespace>
helm upgrade --install <release> . -f integration-tests-values.yaml -n <namespace> --dry-run --debug

# terminal 1
set -o pipefail
until kubectl logs -f job/<job-name> -n <namespace> 2>/dev/null | tee results.txt; do sleep 2; done
# terminal 2
helm upgrade --install <release> . -f integration-tests-values.yaml -n <namespace>
```

The test image derives from `smoke-test-base` (1.0.9 in the 1.3.0 profile), a distroless Python image with no shell: `kubectl exec ... -- /bin/sh` does not work; diagnose from logs.

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `image has non-numeric user (nvs)` | `runAsUser` removed | Restore `runAsUser: 1000` |
| Job never completes, sidecar running | Native sidecar on Kubernetes < 1.29 | Upgrade, or use client credentials |
| Few tests ran | Services not discovered | Read "Discovered Services"; check the catalog or set `manualEndpoints` |
| Zero tests ran | Unknown marker | Use the markers above |
| Preflight fails, services reachable | Probe pattern mismatch | `skipConnectivityChecks: true` |
| `serviceAccount.name is required ...` | `create: false` without a name | Set `serviceAccount.name` |
| `provisionerSidecar.container.image is required` | Token-file mode without sidecar image | Set it or disable the mode |
| `AADSTS70021` / `AADSTS70025` | Federated subject changed | Register the exact subject or reuse the bound SA |
| No pod after a pass | `hook-succeeded` deleted it | Start the watcher before installing |
| Another release's Job vanished | Shared `job.name` | Unique `job.name` |
| Doubled `/api/v1/services` | Path included in `discoveryServiceEndpoint` | Base URL only |

## Released 1.0.18 (previous profile)

The 1.0.18 image inspected by the source package (digest `sha256:fddd63931ed935b2237b9746f628c0e362f93faa9ba9bc9aaa900d7b829b7332`) registers `rest`, `grpc`, `storage_and_permission`, `storage_and_notification`, `storage_and_thumbnail` and `live_edit`, not `permit`/`forbid`. Its chart already uses `serviceIdentity.clientCredentials` / `accessTokenFile`; `job.useTokenSecrets` is obsolete; images are set with `image.repository`/`image.tag`. Check the running image digest and collected markers before changing selectors. Moving to 1.0.23 does not fix or prove coverage on 1.0.18.

## Cleanup

Uninstall only your release after confirming the name and context (`helm uninstall <release> -n <namespace>`). Do not delete the namespace or shared services. Uninstall does not remove objects written under `storageApiResourceBase`; delete that prefix only with explicit authorization and after any retention requirement is met.
