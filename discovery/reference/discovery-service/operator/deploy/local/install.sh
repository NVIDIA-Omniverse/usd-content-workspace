#!/bin/sh
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

set -eu

CONTEXT=${CONTEXT:-docker-desktop}
NAMESPACE=${NAMESPACE:-discovery-routing-test}
CONTOUR_NAMESPACE=${CONTOUR_NAMESPACE:-projectcontour}
SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
ROOT_DIR=$(CDPATH='' cd -- "$SCRIPT_DIR/../.." && pwd)

for command in docker kubectl helm; do
  command -v "$command" >/dev/null 2>&1 || { echo "error: $command is required" >&2; exit 1; }
done

if ! kubectl config get-contexts "$CONTEXT" --no-headers 2>/dev/null | grep -q .; then
  echo "error: kubectl context '$CONTEXT' does not exist" >&2
  exit 1
fi

kubectl --context "$CONTEXT" cluster-info >/dev/null

if [ "${BUILD_IMAGES:-true}" = "true" ]; then
  docker build -t auto-discovery-service:local -f "$ROOT_DIR/Dockerfile" "$ROOT_DIR"
  docker build -t mock-capabilities:local -f "$ROOT_DIR/Dockerfile.mock" "$ROOT_DIR"
fi

if ! kubectl --context "$CONTEXT" get namespace "$CONTOUR_NAMESPACE" >/dev/null 2>&1; then
  echo "Installing pinned Contour v1.33.5..."
  kubectl --context "$CONTEXT" apply -f "$SCRIPT_DIR/contour-v1.33.5.yaml"
fi

kubectl --context "$CONTEXT" -n "$CONTOUR_NAMESPACE" rollout status deployment/contour --timeout=180s
kubectl --context "$CONTEXT" -n "$CONTOUR_NAMESPACE" rollout status daemonset/envoy --timeout=180s

kubectl --context "$CONTEXT" create namespace "$NAMESPACE" --dry-run=client -o yaml | \
  kubectl --context "$CONTEXT" apply -f -
kubectl --context "$CONTEXT" -n "$NAMESPACE" apply -f "$SCRIPT_DIR/mock-capabilities.yaml"

helm upgrade --install discovery-routing "$ROOT_DIR/helm/auto-discovery-service" \
  --kube-context "$CONTEXT" \
  --namespace "$NAMESPACE" \
  --values "$SCRIPT_DIR/values.yaml" \
  --wait \
  --timeout 3m

# A local image keeps the same tag, so rebuilding it does not alter the Pod
# template. Restart explicitly to ensure the cluster runs the new image.
if [ "${BUILD_IMAGES:-true}" = "true" ]; then
  kubectl --context "$CONTEXT" -n "$NAMESPACE" rollout restart deployment/auto-discovery-service
  kubectl --context "$CONTEXT" -n "$NAMESPACE" rollout restart deployment/mock-capabilities
fi

kubectl --context "$CONTEXT" -n "$NAMESPACE" rollout status deployment/mock-capabilities --timeout=180s
kubectl --context "$CONTEXT" -n "$NAMESPACE" rollout status deployment/auto-discovery-service --timeout=180s

"$SCRIPT_DIR/test.sh"
