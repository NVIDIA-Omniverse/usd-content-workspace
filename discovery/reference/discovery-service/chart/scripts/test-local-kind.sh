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

set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HELM_DIR="$ROOT_DIR/helm"
CONFORMANCE_CHART_DIR="$HELM_DIR/discovery-conformance-tests"
LOCAL_TEST_DIR="$ROOT_DIR/tests/local"

CLUSTER_NAME="${CLUSTER_NAME:-discovery-chart-test-$$}"
KUBE_CONTEXT="kind-$CLUSTER_NAME"
NAMESPACE="discovery-test"
RELEASE_NAME="discovery-service"
CONFORMANCE_RELEASE_NAME="discovery-conformance-tests"
DISCOVERY_IMAGE="${DISCOVERY_IMAGE:-simple-nginx-arm64:latest}"
DISCOVERY_LOCAL_IMAGE="discovery-service-local:test"
SMOKE_LOCAL_IMAGE="discovery-smoke-test-local:test"
CONFORMANCE_LOCAL_IMAGE="discovery-conformance-tests-local:test"
KEEP_CLUSTER="${KEEP_CLUSTER:-0}"
REBUILD_TEST_IMAGES="${REBUILD_TEST_IMAGES:-0}"
LOCAL_PORT="${LOCAL_PORT:-18080}"
TIMEOUT="${TIMEOUT:-3m}"

TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/discovery-chart-test.XXXXXX")"
export KUBECONFIG="$TMP_DIR/kubeconfig"
export BUILDX_CONFIG="${BUILDX_CONFIG:-$TMP_DIR/buildx}"
PORT_FORWARD_PID=""
CLUSTER_CREATED=0

info() {
  printf '\n==> %s\n' "$*"
}

die() {
  printf 'ERROR: %s\n' "$*" >&2
  exit 1
}

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

kube() {
  kubectl --context "$KUBE_CONTEXT" "$@"
}

stop_port_forward() {
  if [[ -n "$PORT_FORWARD_PID" ]] && kill -0 "$PORT_FORWARD_PID" 2>/dev/null; then
    kill "$PORT_FORWARD_PID" 2>/dev/null || true
    wait "$PORT_FORWARD_PID" 2>/dev/null || true
  fi
  PORT_FORWARD_PID=""
}

collect_diagnostics() {
  [[ "$CLUSTER_CREATED" -eq 1 ]] || return 0

  info "Failure diagnostics"
  helm status "$RELEASE_NAME" --kube-context "$KUBE_CONTEXT" \
    --namespace "$NAMESPACE" 2>/dev/null || true
  helm status "$CONFORMANCE_RELEASE_NAME" --kube-context "$KUBE_CONTEXT" \
    --namespace "$NAMESPACE" 2>/dev/null || true
  kube --namespace "$NAMESPACE" get all,configmap,serviceaccount -o wide 2>/dev/null || true
  kube --namespace "$NAMESPACE" describe pods 2>/dev/null || true

  while IFS= read -r pod; do
    [[ -n "$pod" ]] || continue
    printf '\n--- logs: %s ---\n' "$pod"
    kube --namespace "$NAMESPACE" logs "$pod" --all-containers --prefix 2>/dev/null || true
  done < <(kube --namespace "$NAMESPACE" get pods -o name 2>/dev/null || true)

  printf '\n--- namespace events ---\n'
  kube --namespace "$NAMESPACE" get events \
    --sort-by=.metadata.creationTimestamp 2>/dev/null || true
}

cleanup() {
  local status=$?
  set +e
  stop_port_forward

  if [[ "$status" -ne 0 ]]; then
    collect_diagnostics
  fi

  if [[ "$CLUSTER_CREATED" -eq 1 ]]; then
    if [[ "$KEEP_CLUSTER" == "1" ]]; then
      info "Keeping kind cluster $CLUSTER_NAME for inspection"
      printf 'export KUBECONFIG=%q\n' "$KUBECONFIG"
      printf 'kubectl --context %s -n %s get all\n' "$KUBE_CONTEXT" "$NAMESPACE"
    else
      info "Deleting kind cluster $CLUSTER_NAME"
      kind delete cluster --name "$CLUSTER_NAME"
    fi
  fi

  if [[ "$KEEP_CLUSTER" != "1" || "$CLUSTER_CREATED" -eq 0 ]]; then
    rm -rf "$TMP_DIR"
  fi
  exit "$status"
}

trap cleanup EXIT
trap 'exit 130' INT TERM

assert_release_owner() {
  local resource=$1
  local owner
  owner="$(kube --namespace "$NAMESPACE" get "$resource" \
    -o jsonpath='{.metadata.annotations.meta\.helm\.sh/release-name}')"
  [[ "$owner" == "$RELEASE_NAME" ]] || {
    die "$resource is owned by '$owner', expected '$RELEASE_NAME'"
  }
}

start_port_forward() {
  local log_file="$TMP_DIR/port-forward.log"
  : >"$log_file"

  kube --namespace "$NAMESPACE" port-forward \
    "service/$RELEASE_NAME" "$LOCAL_PORT:8080" >"$log_file" 2>&1 &
  PORT_FORWARD_PID=$!

  local attempt=0
  until curl --fail --silent --show-error \
      "http://127.0.0.1:$LOCAL_PORT/health" >/dev/null 2>&1; do
    attempt=$((attempt + 1))
    if ! kill -0 "$PORT_FORWARD_PID" 2>/dev/null; then
      awk '{ print }' "$log_file" >&2
      die "kubectl port-forward exited before the service became reachable"
    fi
    if [[ "$attempt" -ge 30 ]]; then
      awk '{ print }' "$log_file" >&2
      die "service did not become reachable through port-forward"
    fi
    sleep 1
  done
}

fetch_runtime_endpoints() {
  local expected_phase=$1
  local health
  local metrics

  health="$(curl --fail --silent --show-error \
    "http://127.0.0.1:$LOCAL_PORT/health")"
  [[ "$health" == "healthy" ]] || die "unexpected /health response: $health"

  metrics="$(curl --fail --silent --show-error \
    "http://127.0.0.1:$LOCAL_PORT/metrics")"
  [[ "$metrics" == *"Active connections"* ]] || {
    die "/metrics did not return NGINX stub status"
  }

  curl --fail --silent --show-error \
    "http://127.0.0.1:$LOCAL_PORT/api/v1alpha/services" \
    >"$TMP_DIR/$expected_phase-v1alpha.json"
  curl --fail --silent --show-error \
    "http://127.0.0.1:$LOCAL_PORT/api/v1/services" \
    >"$TMP_DIR/$expected_phase-v1.json"
}

assert_initial_payloads() {
  python3 - "$TMP_DIR/initial-v1alpha.json" "$TMP_DIR/initial-v1.json" <<'PY'
import json
import pathlib
import sys

v1alpha = json.loads(pathlib.Path(sys.argv[1]).read_text())
v1 = json.loads(pathlib.Path(sys.argv[2]).read_text())
expected = {
    "schema-version": 1,
    "services": [
        {
            "grpc": "http://storage-service.discovery-test.svc.cluster.local:8011/",
            "id": "storage-service-local",
            "name": "Local Storage Service",
            "rest": "http://storage-service.discovery-test.svc.cluster.local:8012/",
            "type": "storage",
        },
        {
            "grpc": "https://grpc.permissions.test:7443/",
            "id": "permission-service-local",
            "name": "Explicit Permission Service",
            "rest": "https://api.permissions.test:8443/v1/check",
            "type": "permission",
        },
        {
            "id": "event-aggregation-service-local",
            "name": "Custom Event Aggregation Service",
            "rest": "http://events.test:8088/events",
            "type": "notification-aggregation",
        },
    ],
}

if v1alpha != v1:
    raise SystemExit("v1alpha and v1 payloads differ")
if v1 != expected:
    raise SystemExit(
        "initial discovery payload mismatch\n"
        f"expected: {json.dumps(expected, indent=2, sort_keys=True)}\n"
        f"actual:   {json.dumps(v1, indent=2, sort_keys=True)}"
    )
PY
}

assert_upgrade_payloads() {
  python3 - "$TMP_DIR/upgrade-v1alpha.json" "$TMP_DIR/upgrade-v1.json" <<'PY'
import json
import pathlib
import sys

v1alpha = json.loads(pathlib.Path(sys.argv[1]).read_text())
v1 = json.loads(pathlib.Path(sys.argv[2]).read_text())
expected = {
    "schema-version": 1,
    "services": [
        {
            "grpc": "http://storage-service.discovery-test.svc.cluster.local:9011/",
            "id": "storage-service-upgraded",
            "name": "Upgraded Storage Service",
            "rest": "https://upgraded-storage.test:9443/api/storage",
            "type": "storage",
        }
    ],
}

if v1alpha != v1:
    raise SystemExit("upgraded v1alpha and v1 payloads differ")
if v1 != expected:
    raise SystemExit(
        "upgraded discovery payload mismatch\n"
        f"expected: {json.dumps(expected, indent=2, sort_keys=True)}\n"
        f"actual:   {json.dumps(v1, indent=2, sort_keys=True)}"
    )
PY
}

assert_no_release_resources() {
  local release=$1
  local remaining
  remaining="$(kube --namespace "$NAMESPACE" get \
    all,configmap,serviceaccount \
    --selector "app.kubernetes.io/instance=$release" \
    -o name 2>/dev/null || true)"
  [[ -z "$remaining" ]] || {
    printf '%s\n' "$remaining" >&2
    die "resources for release $release remain after uninstall"
  }
}

info "Checking prerequisites"
for command_name in docker kind kubectl helm curl python3; do
  require_cmd "$command_name"
done
docker info >/dev/null 2>&1 || die "Docker daemon is not available"
[[ "$KEEP_CLUSTER" == "0" || "$KEEP_CLUSTER" == "1" ]] || {
  die "KEEP_CLUSTER must be 0 or 1"
}
[[ "$REBUILD_TEST_IMAGES" == "0" || "$REBUILD_TEST_IMAGES" == "1" ]] || {
  die "REBUILD_TEST_IMAGES must be 0 or 1"
}
[[ "$CLUSTER_NAME" =~ ^[a-z0-9][a-z0-9.-]*$ ]] || {
  die "CLUSTER_NAME must contain only lowercase letters, numbers, dots, and hyphens"
}
[[ "$LOCAL_PORT" =~ ^[0-9]+$ ]] || die "LOCAL_PORT must be numeric"
((LOCAL_PORT >= 1024 && LOCAL_PORT < 65535)) || {
  die "LOCAL_PORT must be between 1024 and 65534"
}

if kind get clusters | awk -v name="$CLUSTER_NAME" '
  $0 == name { found = 1 }
  END { exit(found ? 0 : 1) }
'; then
  die "kind cluster already exists: $CLUSTER_NAME"
fi

docker image inspect "$DISCOVERY_IMAGE" >/dev/null 2>&1 || {
  die "discovery image '$DISCOVERY_IMAGE' is not local; pull/build it first or set DISCOVERY_IMAGE"
}

info "Linting and rendering charts"
helm lint "$HELM_DIR" --values "$LOCAL_TEST_DIR/values.yaml"
helm lint "$CONFORMANCE_CHART_DIR"
helm template "$RELEASE_NAME-defaults" "$HELM_DIR" \
  --namespace "$NAMESPACE" >"$TMP_DIR/defaults.yaml"
helm template "$RELEASE_NAME" "$HELM_DIR" \
  --namespace "$NAMESPACE" \
  --values "$LOCAL_TEST_DIR/values.yaml" >"$TMP_DIR/local.yaml"

info "Building local test images"
docker tag "$DISCOVERY_IMAGE" "$DISCOVERY_LOCAL_IMAGE"
if [[ "$REBUILD_TEST_IMAGES" == "1" ]] || \
    ! docker image inspect "$SMOKE_LOCAL_IMAGE" >/dev/null 2>&1; then
  docker build \
    --file "$LOCAL_TEST_DIR/smoke.Dockerfile" \
    --tag "$SMOKE_LOCAL_IMAGE" \
    "$LOCAL_TEST_DIR"
else
  printf 'Reusing %s\n' "$SMOKE_LOCAL_IMAGE"
fi
if [[ "$REBUILD_TEST_IMAGES" == "1" ]] || \
    ! docker image inspect "$CONFORMANCE_LOCAL_IMAGE" >/dev/null 2>&1; then
  docker build \
    --tag "$CONFORMANCE_LOCAL_IMAGE" \
    "$ROOT_DIR/conformance_tests"
else
  printf 'Reusing %s\n' "$CONFORMANCE_LOCAL_IMAGE"
fi

info "Creating isolated kind cluster $CLUSTER_NAME"
kind create cluster --name "$CLUSTER_NAME" --wait "$TIMEOUT"
CLUSTER_CREATED=1

info "Loading images into kind"
kind load docker-image \
  "$DISCOVERY_LOCAL_IMAGE" \
  "$SMOKE_LOCAL_IMAGE" \
  "$CONFORMANCE_LOCAL_IMAGE" \
  --name "$CLUSTER_NAME"

info "Running Kubernetes server-side dry-run"
kube create namespace "$NAMESPACE"
kube label namespace "$NAMESPACE" \
  pod-security.kubernetes.io/enforce=baseline \
  pod-security.kubernetes.io/warn=restricted
kube --namespace "$NAMESPACE" apply \
  --dry-run=server \
  --filename "$TMP_DIR/local.yaml" >/dev/null

info "Installing discovery chart"
helm upgrade --install "$RELEASE_NAME" "$HELM_DIR" \
  --kube-context "$KUBE_CONTEXT" \
  --namespace "$NAMESPACE" \
  --values "$LOCAL_TEST_DIR/values.yaml" \
  --wait \
  --timeout "$TIMEOUT"

kube --namespace "$NAMESPACE" rollout status \
  "deployment/$RELEASE_NAME" --timeout="$TIMEOUT"
kube --namespace "$NAMESPACE" wait \
  --for=condition=Ready \
  --selector "app.kubernetes.io/instance=$RELEASE_NAME" \
  pod --timeout="$TIMEOUT"

assert_release_owner "configmap/$RELEASE_NAME"
assert_release_owner "service/$RELEASE_NAME"
assert_release_owner "deployment/$RELEASE_NAME"

info "Checking health, metrics, and discovery payloads"
start_port_forward
fetch_runtime_endpoints initial
assert_initial_payloads

info "Running Helm smoke test"
helm test "$RELEASE_NAME" \
  --kube-context "$KUBE_CONTEXT" \
  --namespace "$NAMESPACE" \
  --logs \
  --timeout "$TIMEOUT"
kube --namespace "$NAMESPACE" delete \
  "pod/$RELEASE_NAME-tests" --ignore-not-found

info "Running in-cluster conformance suite"
helm upgrade --install "$CONFORMANCE_RELEASE_NAME" "$CONFORMANCE_CHART_DIR" \
  --kube-context "$KUBE_CONTEXT" \
  --namespace "$NAMESPACE" \
  --set-string fullnameOverride="$CONFORMANCE_RELEASE_NAME" \
  --set-string image.repository=discovery-conformance-tests-local \
  --set-string image.tag=test \
  --set image.pullPolicy=Never \
  --set-string service.restEndpoint="http://$RELEASE_NAME:8080" \
  --set-string service.healthEndpoint="http://$RELEASE_NAME:8080/health" \
  --wait \
  --wait-for-jobs \
  --timeout "$TIMEOUT"
kube --namespace "$NAMESPACE" wait \
  --for=condition=Complete \
  "job/$CONFORMANCE_RELEASE_NAME" --timeout="$TIMEOUT"
kube --namespace "$NAMESPACE" logs \
  "job/$CONFORMANCE_RELEASE_NAME" --all-containers

info "Upgrading service catalog and checking rollout"
old_pod_name="$(kube --namespace "$NAMESPACE" get pods \
  --selector "app.kubernetes.io/instance=$RELEASE_NAME" \
  -o jsonpath='{.items[0].metadata.name}')"
old_pod_uid="$(kube --namespace "$NAMESPACE" get pods \
  --selector "app.kubernetes.io/instance=$RELEASE_NAME" \
  -o jsonpath='{.items[0].metadata.uid}')"
old_checksum="$(kube --namespace "$NAMESPACE" get "deployment/$RELEASE_NAME" \
  -o jsonpath='{.spec.template.metadata.annotations.checksum/config}')"

helm upgrade "$RELEASE_NAME" "$HELM_DIR" \
  --kube-context "$KUBE_CONTEXT" \
  --namespace "$NAMESPACE" \
  --values "$LOCAL_TEST_DIR/values.yaml" \
  --values "$LOCAL_TEST_DIR/values-upgrade.yaml" \
  --wait \
  --timeout "$TIMEOUT"
kube --namespace "$NAMESPACE" rollout status \
  "deployment/$RELEASE_NAME" --timeout="$TIMEOUT"
kube --namespace "$NAMESPACE" wait \
  --for=delete "pod/$old_pod_name" --timeout="$TIMEOUT"

new_pod_uid="$(kube --namespace "$NAMESPACE" get pods \
  --selector "app.kubernetes.io/instance=$RELEASE_NAME" \
  -o jsonpath='{.items[0].metadata.uid}')"
new_checksum="$(kube --namespace "$NAMESPACE" get "deployment/$RELEASE_NAME" \
  -o jsonpath='{.spec.template.metadata.annotations.checksum/config}')"
[[ "$old_pod_uid" != "$new_pod_uid" ]] || die "upgrade did not replace the pod"
[[ "$old_checksum" != "$new_checksum" ]] || die "upgrade did not change the config checksum"

stop_port_forward
LOCAL_PORT=$((LOCAL_PORT + 1))
start_port_forward
fetch_runtime_endpoints upgrade
assert_upgrade_payloads

info "Uninstalling releases and checking cleanup"
stop_port_forward
helm uninstall "$CONFORMANCE_RELEASE_NAME" \
  --kube-context "$KUBE_CONTEXT" \
  --namespace "$NAMESPACE" \
  --wait
helm uninstall "$RELEASE_NAME" \
  --kube-context "$KUBE_CONTEXT" \
  --namespace "$NAMESPACE" \
  --wait
kube --namespace "$NAMESPACE" wait \
  --for=delete pod \
  --selector "app.kubernetes.io/instance=$CONFORMANCE_RELEASE_NAME" \
  --timeout="$TIMEOUT"
kube --namespace "$NAMESPACE" wait \
  --for=delete pod \
  --selector "app.kubernetes.io/instance=$RELEASE_NAME" \
  --timeout="$TIMEOUT"
assert_no_release_resources "$CONFORMANCE_RELEASE_NAME"
assert_no_release_resources "$RELEASE_NAME"

info "Local Kubernetes deployment test passed"
