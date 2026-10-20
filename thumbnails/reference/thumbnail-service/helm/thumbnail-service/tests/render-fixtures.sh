#!/usr/bin/env bash
# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

# Chart render fixtures for helm-common-service-worker-values-split.
# Run from repo root: bash helm/thumbnail-service/tests/render-fixtures.sh
set -euo pipefail

CHART="$(cd "$(dirname "$0")/.." && pwd)"
ROOT="$(cd "$CHART/../.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

COMMON=(
  --set thumbnailPathTemplate='https://example.blob.core.windows.net/c/{resource_address_md5}/{width}x{height}/{resource_filename}.{output_encoding}'
  --set thumbnailSizes=256x256
  --set notifications.eventConsumer.endpoint=notifications:50051
  --set 'notifications.eventConsumer.resourcePrefixes[0]=https://account.blob.core.windows.net/container/'
  --set serviceIdentity.accessTokenFile.enabled=true
  --set serviceIdentity.accessTokenFile.provisionerSidecar.container.image=sidecar:latest
)

fail() { echo "FAIL: $*" >&2; exit 1; }
pass() { echo "PASS: $*"; }

# 4.4a: storage.endpoint required when worker enabled
if helm template t "$CHART" "${COMMON[@]}" \
  --set storage.endpoint= \
  >"$TMP/out.yaml" 2>"$TMP/err.txt"; then
  fail "expected storage.endpoint required-field failure"
fi
grep -q "storage.endpoint is required" "$TMP/err.txt" \
  || fail "error message missing storage.endpoint (got: $(cat "$TMP/err.txt"))"
pass "storage.endpoint required-field failure"

# 4.4b: thumbnailWorker.enabled=false → orchestrator-only, no worker resources
helm template t "$CHART" "${COMMON[@]}" \
  --set thumbnailWorker.enabled=false \
  --set storage.endpoint=storage:50001 \
  >"$TMP/orch-only.yaml"
grep -q "kind: Deployment" "$TMP/orch-only.yaml" || fail "orchestrator Deployment missing"
grep -q "name: thumbnail-service" "$TMP/orch-only.yaml" || fail "orchestrator name missing"
if grep -q "thumbnail-task-worker\|thumbnail-worker\|component: task-worker" "$TMP/orch-only.yaml"; then
  fail "worker resources rendered with thumbnailWorker.enabled=false"
fi
pass "thumbnailWorker.enabled=false orchestrator-only install"

# Client-credentials mode mounts an existing Secret; the chart does not emit Secret objects
helm template t "$CHART" \
  --set thumbnailPathTemplate='https://example.blob.core.windows.net/c/{resource_address_md5}/{width}x{height}/{resource_filename}.{output_encoding}' \
  --set thumbnailSizes=256x256 \
  --set notifications.eventConsumer.endpoint=notifications:50051 \
  --set 'notifications.eventConsumer.resourcePrefixes[0]=https://account.blob.core.windows.net/container/' \
  --set storage.endpoint=storage:50001 \
  --set serviceIdentity.clientCredentials.enabled=true \
  --set serviceIdentity.clientCredentials.clientId=cid \
  --set serviceIdentity.clientCredentials.clientScope=scope \
  --set serviceIdentity.clientCredentials.openIdConfigurationUri=http://idp/.well-known/openid-configuration \
  --set serviceIdentity.clientCredentials.clientSecretRef.name=external-oauth-secret \
  >"$TMP/cc-secret.yaml"
secret_count="$(grep -c '^kind: Secret$' "$TMP/cc-secret.yaml" || true)"
[[ "$secret_count" -eq 0 ]] || fail "expected 0 Secrets from the chart, got $secret_count"
grep -A2 'name: worker-identity-secret' "$TMP/cc-secret.yaml" | grep -q 'secretName: external-oauth-secret' \
  || fail "worker volume did not mount external-oauth-secret"
grep -A2 'name: service-identity-secret' "$TMP/cc-secret.yaml" | grep -q 'secretName: external-oauth-secret' \
  || fail "orchestrator volume did not mount external-oauth-secret"
pass "client-credentials mounts existing Secret; chart emits none"

# clientSecretRef.name required in client-credentials mode
if helm template t "$CHART" \
  --set thumbnailPathTemplate='https://example.blob.core.windows.net/c/{resource_address_md5}/{width}x{height}/{resource_filename}.{output_encoding}' \
  --set thumbnailSizes=256x256 \
  --set notifications.eventConsumer.endpoint=notifications:50051 \
  --set 'notifications.eventConsumer.resourcePrefixes[0]=https://account.blob.core.windows.net/container/' \
  --set storage.endpoint=storage:50001 \
  --set serviceIdentity.clientCredentials.enabled=true \
  --set serviceIdentity.clientCredentials.clientId=cid \
  --set serviceIdentity.clientCredentials.clientScope=scope \
  --set serviceIdentity.clientCredentials.openIdConfigurationUri=http://idp/.well-known/openid-configuration \
  --set thumbnailWorker.enabled=false \
  >"$TMP/out.yaml" 2>"$TMP/err.txt"; then
  fail "expected missing clientSecretRef.name failure"
fi
grep -q "clientSecretRef.name" "$TMP/err.txt" \
  || fail "error message missing clientSecretRef.name (got: $(cat "$TMP/err.txt"))"
pass "clientSecretRef.name required when clientCredentials is enabled"

# Probe ports honor values (not hard-wired to service.restPort)
helm template t "$CHART" "${COMMON[@]}" \
  --set thumbnailWorker.enabled=false \
  --set storage.endpoint=storage:50001 \
  --set thumbnailService.startupProbe.httpGet.port=3001 \
  --set thumbnailService.livenessProbe.httpGet.port=3002 \
  --set thumbnailService.readinessProbe.httpGet.port=3003 \
  >"$TMP/probe-ports.yaml"
grep -A3 'startupProbe:' "$TMP/probe-ports.yaml" | grep -q 'port: 3001' \
  || fail "startupProbe port not honored"
grep -A3 'livenessProbe:' "$TMP/probe-ports.yaml" | grep -q 'port: 3002' \
  || fail "livenessProbe port not honored"
grep -A3 'readinessProbe:' "$TMP/probe-ports.yaml" | grep -q 'port: 3003' \
  || fail "readinessProbe port not honored"
pass "probe httpGet.port values are honored"

# Invalid Secret key rejected early (Kubernetes data-key charset)
if helm template t "$CHART" \
  --set thumbnailPathTemplate='https://example.blob.core.windows.net/c/{resource_address_md5}/{width}x{height}/{resource_filename}.{output_encoding}' \
  --set thumbnailSizes=256x256 \
  --set notifications.eventConsumer.endpoint=notifications:50051 \
  --set 'notifications.eventConsumer.resourcePrefixes[0]=https://account.blob.core.windows.net/container/' \
  --set storage.endpoint=storage:50001 \
  --set serviceIdentity.clientCredentials.enabled=true \
  --set serviceIdentity.clientCredentials.clientId=cid \
  --set serviceIdentity.clientCredentials.clientScope=scope \
  --set serviceIdentity.clientCredentials.openIdConfigurationUri=http://idp/.well-known/openid-configuration \
  --set serviceIdentity.clientCredentials.clientSecretRef.key='bad/key' \
  --set serviceIdentity.clientCredentials.clientSecretRef.name=external-oauth-secret \
  --set thumbnailWorker.enabled=false \
  >"$TMP/out.yaml" 2>"$TMP/err.txt"; then
  fail "expected invalid clientSecretRef.key failure"
fi
grep -q "not a valid Kubernetes Secret key" "$TMP/err.txt" \
  || fail "error message missing Secret key pattern (got: $(cat "$TMP/err.txt"))"
pass "invalid clientSecretRef.key rejected"

# A single prefix written as a bare string renders the same on both Deployments.
# `make create-local-values` writes exactly this shape.
helm template t "$CHART" \
  --set thumbnailPathTemplate='https://example.blob.core.windows.net/c/{resource_address_md5}/{width}x{height}/{resource_filename}.{output_encoding}' \
  --set thumbnailSizes=256x256 \
  --set notifications.eventConsumer.endpoint=notifications:50051 \
  --set-string notifications.eventConsumer.resourcePrefixes=http://172.18.0.1:50001/local/ \
  --set serviceIdentity.accessTokenFile.enabled=true \
  --set serviceIdentity.accessTokenFile.provisionerSidecar.container.image=sidecar:latest \
  --set storage.endpoint=storage:50001 \
  >"$TMP/scalar-prefixes.yaml" 2>"$TMP/err.txt" \
  || fail "scalar resourcePrefixes failed to render (got: $(cat "$TMP/err.txt"))"
scalar_hits="$(grep -c 'value: "http://172.18.0.1:50001/local/"' "$TMP/scalar-prefixes.yaml" || true)"
[[ "$scalar_hits" -eq 2 ]] \
  || fail "expected the scalar prefix on both Deployments, got $scalar_hits"
pass "scalar resourcePrefixes renders on both Deployments"

# Shared chart-level values feed both Deployments while repositories and
# ServiceAccount names remain component-specific.
helm template t "$CHART" "${COMMON[@]}" \
  --set storage.endpoint=storage:50001 \
  --set image.tag=shared-tag \
  --set image.pullPolicy=Always \
  --set image.pullSecrets[0].name=shared-registry \
  --set serviceAccount.automount=true \
  --set podLabels.shared-label=true \
  --set podAnnotations.shared-annotation=true \
  --set nodeSelector.disktype=ssd \
  >"$TMP/shared-root.yaml" 2>"$TMP/err.txt" \
  || fail "shared root values failed to render (got: $(cat "$TMP/err.txt"))"
grep -q 'image: "nvcr.io/nvidia/omniverse/thumbnail-service:shared-tag"' "$TMP/shared-root.yaml" \
  || fail "orchestrator did not use root image.tag"
grep -q 'image: "nvcr.io/nvidia/omniverse/thumbnail-task-worker:shared-tag"' "$TMP/shared-root.yaml" \
  || fail "worker did not use root image.tag"
[[ "$(grep -c 'imagePullPolicy: Always' "$TMP/shared-root.yaml" || true)" -eq 2 ]] \
  || fail "root image.pullPolicy did not reach both Deployments"
[[ "$(grep -c 'name: shared-registry' "$TMP/shared-root.yaml" || true)" -eq 2 ]] \
  || fail "root image.pullSecrets did not reach both Deployments"
[[ "$(grep -c 'automountServiceAccountToken: true' "$TMP/shared-root.yaml" || true)" -ge 4 ]] \
  || fail "root serviceAccount.automount did not reach both ServiceAccounts and Deployments"
[[ "$(grep -c 'shared-label: true' "$TMP/shared-root.yaml" || true)" -eq 2 ]] \
  || fail "root podLabels did not reach both Deployments"
[[ "$(grep -c 'shared-annotation: true' "$TMP/shared-root.yaml" || true)" -eq 2 ]] \
  || fail "root podAnnotations did not reach both Deployments"
[[ "$(grep -c 'disktype: ssd' "$TMP/shared-root.yaml" || true)" -eq 2 ]] \
  || fail "root nodeSelector did not reach both Deployments"
grep -q 'name: THUMBNAIL_SERVICE_GRPC_ENDPOINT' "$TMP/shared-root.yaml" \
  || fail "worker THUMBNAIL_SERVICE_GRPC_ENDPOINT missing"
grep -A1 'name: THUMBNAIL_SERVICE_GRPC_ENDPOINT' "$TMP/shared-root.yaml" \
  | grep -q 'value: "thumbnail-service:50051"' \
  || fail "worker associate endpoint was not derived from the in-cluster Service"
pass "shared root values feed both Deployments"

# Associate endpoint follows service.nameOverride / service.grpcPort
helm template t "$CHART" "${COMMON[@]}" \
  --set storage.endpoint=storage:50001 \
  --set service.nameOverride=thumb-svc \
  --set service.grpcPort=15051 \
  >"$TMP/derived-grpc.yaml" 2>"$TMP/err.txt" \
  || fail "derived associate endpoint failed to render (got: $(cat "$TMP/err.txt"))"
grep -A1 'name: THUMBNAIL_SERVICE_GRPC_ENDPOINT' "$TMP/derived-grpc.yaml" \
  | grep -q 'value: "thumb-svc:15051"' \
  || fail "associate endpoint did not follow service.nameOverride/grpcPort"
pass "associate endpoint follows in-cluster Service"

# Both StorageEventQueue's and WorkerEventQueue's ids are derived at runtime by
# their own binary from notifications.eventConsumer.resourcePrefixes, not
# injected by Helm; there is no values key for either id.
helm template t "$CHART" "${COMMON[@]}" \
  --set storage.endpoint=storage:50001 \
  >"$TMP/queue-ids.yaml" 2>"$TMP/err.txt" \
  || fail "queue ids failed to render (got: $(cat "$TMP/err.txt"))"
if grep -q 'name: NOTIFICATIONS_QUEUE_ID' "$TMP/queue-ids.yaml"; then
  fail "neither Deployment should inject NOTIFICATIONS_QUEUE_ID, got: $(grep -A1 'name: NOTIFICATIONS_QUEUE_ID' "$TMP/queue-ids.yaml")"
fi
pass "neither Deployment injects NOTIFICATIONS_QUEUE_ID"

# Print just the Service document out of a rendered manifest. Every assertion below is
# scoped through this: grepping the whole manifest would let the same annotation on some
# other object stand in for the Service actually carrying it. `kind: Service` is matched
# with its newline so `kind: ServiceAccount` does not qualify.
service_doc() {
  RENDERED="$1" python3 - <<'PY'
import os
import sys
from pathlib import Path

for doc in Path(os.environ["RENDERED"]).read_text().split("---"):
    if "kind: Service\n" in doc:
        print(doc)
        sys.exit(0)
sys.exit("no Service object rendered")
PY
}

# Discovery: the operator finds this Service by the annotation and dials the port
# named `grpc`. Both halves are pinned, because losing either one makes the service
# undiscoverable while every other test still passes.
helm template t "$CHART" "${COMMON[@]}" \
  --set storage.endpoint=storage:50001 \
  >"$TMP/discovery.yaml" 2>"$TMP/err.txt" \
  || fail "default render failed (got: $(cat "$TMP/err.txt"))"
service_doc "$TMP/discovery.yaml" >"$TMP/discovery-service.yaml" \
  || fail "could not isolate the Service object from the default render"
grep -q 'discovery.omniverse.nvidia.com/enabled: "true"' "$TMP/discovery-service.yaml" \
  || fail "Service is not annotated for discovery: $(cat "$TMP/discovery-service.yaml")"
grep -q -- '- name: grpc' "$TMP/discovery-service.yaml" \
  || fail "Service has no port named grpc to dial: $(cat "$TMP/discovery-service.yaml")"
pass "Service is annotated for discovery and exposes a grpc-named port"

# An environment can add annotations and narrow visibility without losing the default.
helm template t "$CHART" "${COMMON[@]}" \
  --set storage.endpoint=storage:50001 \
  --set 'service.annotations.discovery\.omniverse\.nvidia\.com/visibility=internal' \
  >"$TMP/discovery-override.yaml" 2>"$TMP/err.txt" \
  || fail "annotation override failed to render (got: $(cat "$TMP/err.txt"))"
service_doc "$TMP/discovery-override.yaml" >"$TMP/discovery-override-service.yaml" \
  || fail "could not isolate the Service object from the override render"
grep -q 'discovery.omniverse.nvidia.com/visibility: internal' "$TMP/discovery-override-service.yaml" \
  || fail "service.annotations override did not reach the Service"
grep -q 'discovery.omniverse.nvidia.com/enabled: "true"' "$TMP/discovery-override-service.yaml" \
  || fail "service.annotations override dropped the default discovery annotation"
pass "service.annotations merges over the discovery defaults"

# ServiceAccount name resolution. Print the created ServiceAccount names and what each
# Deployment actually references, so a fixture can assert the two agree — a pod naming a
# ServiceAccount that the release never creates still renders fine and only fails once
# it is applied.
sa_wiring() {
  RENDERED="$1" python3 - <<'PY'
import os
from pathlib import Path

created, referenced = [], []
for doc in Path(os.environ["RENDERED"]).read_text().split("---"):
    if "kind: ServiceAccount\n" in doc:
        created += [l.split("name:")[1].strip() for l in doc.splitlines()
                    if l.strip().startswith("name:")][:1]
    if "kind: Deployment\n" in doc:
        referenced += [l.split("serviceAccountName:")[1].strip()
                       for l in doc.splitlines() if "serviceAccountName:" in l]
print("created:", " ".join(sorted(created)) or "-")
print("referenced:", " ".join(sorted(referenced)) or "-")
PY
}

# A values file that sets only the shared name used to land both pods on `default`,
# because the name helpers read the component keys and never the root one. With
# create=false that means running as the namespace default, with none of the Workload
# Identity annotations the deployment was relying on.
helm template t "$CHART" "${COMMON[@]}" \
  --set storage.endpoint=storage:50001 \
  --set serviceAccount.create=false \
  --set serviceAccount.name=sapi-thumbnails \
  >"$TMP/sa-root-nocreate.yaml" 2>"$TMP/err.txt" \
  || fail "shared serviceAccount.name failed to render (got: $(cat "$TMP/err.txt"))"
wiring="$(sa_wiring "$TMP/sa-root-nocreate.yaml")"
[[ "$wiring" == *"referenced: sapi-thumbnails sapi-thumbnails"* ]] \
  || fail "shared serviceAccount.name was not applied to both components: $wiring"
[[ "$wiring" == *"created: -"* ]] \
  || fail "create=false must render no ServiceAccount object: $wiring"
pass "shared serviceAccount.name reaches both components"

# The reason to share a name is a Workload Identity binding keyed to one name, so with
# create=true exactly one object must be rendered — two would collide on the same name.
helm template t "$CHART" "${COMMON[@]}" \
  --set storage.endpoint=storage:50001 \
  --set serviceAccount.name=sapi-thumbnails \
  >"$TMP/sa-root-create.yaml" 2>"$TMP/err.txt" \
  || fail "shared serviceAccount.name with create failed to render (got: $(cat "$TMP/err.txt"))"
wiring="$(sa_wiring "$TMP/sa-root-create.yaml")"
[[ "$wiring" == *"created: sapi-thumbnails"$'\n'* ]] \
  || fail "expected exactly one ServiceAccount object for a shared name: $wiring"
[[ "$wiring" == *"referenced: sapi-thumbnails sapi-thumbnails"* ]] \
  || fail "both components should reference the shared ServiceAccount: $wiring"
pass "a shared serviceAccount.name renders exactly one ServiceAccount"

# Per-component names still win, and each is created — this is the shape dev uses to
# keep the worker on the orchestrator's federated-credential binding.
helm template t "$CHART" "${COMMON[@]}" \
  --set storage.endpoint=storage:50001 \
  --set serviceAccount.name=root-sa \
  --set thumbnailWorker.serviceAccount.name=worker-sa \
  >"$TMP/sa-override.yaml" 2>"$TMP/err.txt" \
  || fail "component serviceAccount override failed to render (got: $(cat "$TMP/err.txt"))"
wiring="$(sa_wiring "$TMP/sa-override.yaml")"
[[ "$wiring" == *"created: root-sa worker-sa"* ]] \
  || fail "expected both the root-named and worker-named ServiceAccounts: $wiring"
[[ "$wiring" == *"referenced: root-sa worker-sa"* ]] \
  || fail "component override did not win over the shared name: $wiring"
pass "a component serviceAccount.name overrides the shared one"

# Default naming is untouched: each component gets its own generated ServiceAccount,
# and each Deployment references the one that exists.
helm template t "$CHART" "${COMMON[@]}" \
  --set storage.endpoint=storage:50001 \
  >"$TMP/sa-default.yaml" 2>"$TMP/err.txt" \
  || fail "default render failed (got: $(cat "$TMP/err.txt"))"
wiring="$(sa_wiring "$TMP/sa-default.yaml")"
[[ "$wiring" == *"created: t-thumbnail-service t-thumbnail-task-worker"* ]] \
  || fail "default ServiceAccount names changed: $wiring"
[[ "$wiring" == *"referenced: t-thumbnail-service t-thumbnail-task-worker"* ]] \
  || fail "default Deployments do not reference their own ServiceAccounts: $wiring"
pass "default per-component ServiceAccount naming is unchanged"

# Default orchestrator resources must land on the thumbnail-service container,
# not only the token sidecar (empty resources: {} made k9s report 10m/32Mi).
helm template t "$CHART" "${COMMON[@]}" \
  --set storage.endpoint=storage:50001 \
  --set thumbnailWorker.enabled=false \
  >"$TMP/orch-resources.yaml" 2>"$TMP/err.txt" \
  || fail "orchestrator resources render failed (got: $(cat "$TMP/err.txt"))"
RENDERED="$TMP/orch-resources.yaml" python3 - <<'PY' || fail "thumbnail-service container missing default resources"
import os, sys
from pathlib import Path

docs = Path(os.environ["RENDERED"]).read_text().split("---")
found = False
for doc in docs:
    if "kind: Deployment\n" not in doc:
        continue
    lines = doc.splitlines()
    in_main = False
    block = []
    for i, line in enumerate(lines):
        if line.strip() == "- name: thumbnail-service":
            in_main = True
            block = [line]
            continue
        if in_main:
            if line.startswith("        - name:") and "thumbnail-service" not in line:
                break
            block.append(line)
    text_lines = [ln.strip() for ln in block]
    if (
        "cpu: 250m" in text_lines
        and "memory: 64Mi" in text_lines
        and "cpu: 1" in text_lines
        and "memory: 256Mi" in text_lines
    ):
        found = True
        break
sys.exit(0 if found else 1)
PY
pass "orchestrator default resources render on the thumbnail-service container"

# Worker memory limit must cover the resample peak, not just the decode: the
# working buffer is source_width x new_height at 16 B/px on top of both images.
helm template t "$CHART" "${COMMON[@]}" \
  --set storage.endpoint=storage:50001 \
  >"$TMP/worker-resources.yaml" 2>"$TMP/err.txt" \
  || fail "worker resources render failed (got: $(cat "$TMP/err.txt"))"
RENDERED="$TMP/worker-resources.yaml" python3 - <<'PY' || fail "thumbnail-worker container missing 2304Mi memory limit"
import os, sys
from pathlib import Path

docs = Path(os.environ["RENDERED"]).read_text().split("---")
found = False
for doc in docs:
    if "kind: Deployment\n" not in doc:
        continue
    lines = doc.splitlines()
    in_main = False
    block = []
    for line in lines:
        if line.strip() == "- name: thumbnail-worker":
            in_main = True
            block = [line]
            continue
        if in_main:
            if line.startswith("        - name:") and "thumbnail-worker" not in line:
                break
            block.append(line)
    text_lines = [ln.strip() for ln in block]
    if (
        "cpu: 1300m" in text_lines
        and "memory: 750Mi" in text_lines
        and "cpu: 2500m" in text_lines
        and "memory: 2304Mi" in text_lines
    ):
        found = True
        break
sys.exit(0 if found else 1)
PY
pass "worker default resources render 1300m/2500m CPU and 2304Mi memory limit on thumbnail-worker"

# An undersized limit must fail rather than install. 1536Mi cannot hold
# 2 x 1 GiB + 200Mi idle.
if helm template t "$CHART" "${COMMON[@]}" \
  --set storage.endpoint=storage:50001 \
  --set thumbnailWorker.resources.limits.memory=1536Mi \
  >"$TMP/out.yaml" 2>"$TMP/err.txt"; then
  fail "expected worker memory-limit failure for 1536Mi"
fi
grep -q "thumbnailWorker.resources.limits.memory" "$TMP/err.txt" \
  || fail "error should name the memory limit (got: $(cat "$TMP/err.txt"))"
grep -q "maxConcurrentConversions" "$TMP/err.txt" \
  || fail "error should name maxConcurrentConversions (got: $(cat "$TMP/err.txt"))"
pass "undersized worker memory limit fails at render"

# Raising concurrency without the limit is the same class of error.
if helm template t "$CHART" "${COMMON[@]}" \
  --set storage.endpoint=storage:50001 \
  --set workerRuntime.maxConcurrentConversions=8 \
  >"$TMP/out.yaml" 2>"$TMP/err.txt"; then
  fail "expected worker memory-limit failure for concurrency 8 at 2304Mi"
fi
grep -q "maxConcurrentConversions" "$TMP/err.txt" \
  || fail "concurrency-8 error should name the knob (got: $(cat "$TMP/err.txt"))"
pass "raising maxConcurrentConversions without a larger limit fails at render"

# Zero concurrency would make the flatten peak look like idle-only (200Mi) and
# pass the limit check. The binary already refuses it (clap range 1..); Helm
# should too, rather than install a worker that converts nothing.
if helm template t "$CHART" "${COMMON[@]}" \
  --set storage.endpoint=storage:50001 \
  --set workerRuntime.maxConcurrentConversions=0 \
  >"$TMP/out.yaml" 2>"$TMP/err.txt"; then
  fail "expected failure for maxConcurrentConversions=0"
fi
grep -q "maxConcurrentConversions must be > 0" "$TMP/err.txt" \
  || fail "zero-concurrency error should name the knob (got: $(cat "$TMP/err.txt"))"
pass "maxConcurrentConversions=0 fails at render"

if helm template t "$CHART" "${COMMON[@]}" \
  --set storage.endpoint=storage:50001 \
  --set workerRuntime.maxDecodedPixels=0 \
  >"$TMP/out.yaml" 2>"$TMP/err.txt"; then
  fail "expected failure for maxDecodedPixels=0"
fi
grep -q "maxDecodedPixels must be > 0" "$TMP/err.txt" \
  || fail "zero-pixels error should name the knob (got: $(cat "$TMP/err.txt"))"
pass "maxDecodedPixels=0 fails at render"

# The pre-restructure observability block must fail the render rather than be
# ignored. Helm drops unrecognized keys silently, so an environment values file
# still on observability.otel.* would otherwise install cleanly and export
# nothing at all.
if helm template t "$CHART" "${COMMON[@]}" \
  --set storage.endpoint=storage:50001 \
  --set observability.otel.endpoint=http://otel:4317 \
  >"$TMP/out.yaml" 2>"$TMP/err.txt"; then
  fail "expected failure for the removed observability.otel block"
fi
grep -q "observability.tracing.endpoint.grpc" "$TMP/err.txt" \
  || fail "observability.otel error should name the replacement keys (got: $(cat "$TMP/err.txt"))"
pass "legacy observability.otel.* fails at render"

# Each signal is gated on its own enabled flag AND a non-empty endpoint, and
# injects the per-signal endpoint variable.
helm template t "$CHART" "${COMMON[@]}" \
  --set storage.endpoint=storage:50001 \
  --set observability.tracing.endpoint.grpc=http://tempo:4317 \
  >"$TMP/tracing-only.yaml"
grep -q "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT" "$TMP/tracing-only.yaml" \
  || fail "tracing endpoint variable missing"
grep -A1 "name: OTEL_METRICS_EXPORTER" "$TMP/tracing-only.yaml" | grep -q '"none"' \
  || fail "metrics exporter should be none when its endpoint is empty"
pass "tracing and metrics are independently gated"

# Both HPAs must key on their own container, so a sidecar carrying no request
# cannot hold the Deployment at minReplicas (the FailedGetResourceMetric case).
helm template t "$CHART" "${COMMON[@]}" \
  --set storage.endpoint=storage:50001 \
  --set thumbnailService.autoscaling.enabled=true \
  --set thumbnailWorker.autoscaling.enabled=true \
  >"$TMP/hpa.yaml"
grep -q "type: ContainerResource" "$TMP/hpa.yaml" || fail "HPAs should use ContainerResource"
if grep -q "^      - type: Resource$" "$TMP/hpa.yaml"; then
  fail "pod-aggregate Resource metric still present"
fi
grep -q "container: thumbnail-service" "$TMP/hpa.yaml" || fail "orchestrator HPA container missing"
grep -q "container: thumbnail-worker" "$TMP/hpa.yaml" || fail "worker HPA container missing"
# The named containers must actually exist, or the HPA ignores those pods silently.
for c in thumbnail-service thumbnail-worker; do
  grep -q "^        - name: $c$" "$TMP/hpa.yaml" || fail "no container named $c in the rendered Deployments"
done
pass "both HPAs target their own container by name"

# The operator-supplied sidecar inherits the shared hardened securityContext.
grep -q "seccompProfile" "$TMP/hpa.yaml" || fail "seccompProfile missing from podSecurityContext"
pass "hardened security defaults render"

# Every rendered document must carry apiVersion. `helm lint` and `helm template`
# both accept a manifest without one, so this escaped into a cluster once: the
# SPDX header is YAML comments, and a following `{{- ... -}}` chain trimmed the
# newline on both sides and appended `apiVersion: v1` to the last comment line.
# Helm rendered it happily; the API server rejected the release with
# "error validating data: apiVersion not set". Assert it across the value
# permutations that change which templates render.
assert_all_docs_have_apiversion() {
  local label="$1" file="$2"
  python3 - "$label" "$file" <<'PY'
import sys
label, path = sys.argv[1], sys.argv[2]
bad = []
for i, doc in enumerate(open(path).read().split("\n---\n")):
    body = [l for l in doc.splitlines() if l.strip() and not l.strip().startswith("#")]
    if not body:
        continue
    if not any(l.startswith("apiVersion:") for l in body):
        kind = next((l for l in body if l.startswith("kind:")), f"document {i}")
        bad.append(kind.strip())
if bad:
    print(f"FAIL: {label}: rendered document(s) without apiVersion: {bad}", file=sys.stderr)
    sys.exit(1)
PY
}

helm template t "$CHART" "${COMMON[@]}" \
  --set storage.endpoint=storage:50001 \
  --set httpProxy.enabled=true \
  --set thumbnailService.autoscaling.enabled=true \
  --set thumbnailWorker.autoscaling.enabled=true \
  --set smokeTest.enabled=true \
  >"$TMP/all-on.yaml"
assert_all_docs_have_apiversion "all features enabled" "$TMP/all-on.yaml" || exit 1

helm template t "$CHART" "${COMMON[@]}" \
  --set storage.endpoint=storage:50001 \
  >"$TMP/defaults.yaml"
assert_all_docs_have_apiversion "default values" "$TMP/defaults.yaml" || exit 1

helm template t "$CHART" "${COMMON[@]}" \
  --set storage.endpoint=storage:50001 \
  --set thumbnailWorker.enabled=false \
  >"$TMP/no-worker.yaml"
assert_all_docs_have_apiversion "worker disabled" "$TMP/no-worker.yaml" || exit 1
pass "every rendered document declares apiVersion"

echo "All helm render fixtures passed."
