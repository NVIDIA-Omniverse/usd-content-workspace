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

SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
ROOT_DIR=$(CDPATH='' cd -- "$SCRIPT_DIR/../.." && pwd)
CHART=$ROOT_DIR/helm/auto-discovery-service
TMP_DIR=${TMPDIR:-/tmp}/discovery-helm-tests
mkdir -p "$TMP_DIR"

helm template routing-test "$CHART" --namespace discovery-routing-test \
  --values "$SCRIPT_DIR/values.yaml" >"$TMP_DIR/rendered.yaml"

routes=$(yq 'select(.kind == "HTTPProxy") | .spec.routes' "$TMP_DIR/rendered.yaml")
printf '%s\n' "$routes" | grep -q 'prefix: /$'
printf '%s\n' "$routes" | grep -q 'name: auto-discovery-service'
printf '%s\n' "$routes" | grep -q 'port: 8081'
if printf '%s\n' "$routes" | grep -q 'exact: "/api/v1'; then
  echo "error: integrated v1 endpoints must use the external catch-all route" >&2
  exit 1
fi

echo "PASS integrated v1/v2 endpoints use the external catch-all route"

helm template namespace-test "$CHART" --namespace current-namespace \
  >"$TMP_DIR/default-namespace.out"
default_namespace_env=$(yq 'select(.kind == "Deployment") | .spec.template.spec.containers[0].env[] | select(.name == "KUBE_NAMESPACE")' "$TMP_DIR/default-namespace.out")
printf '%s\n' "$default_namespace_env" | grep -q 'fieldPath: metadata.namespace'
echo "PASS kubeNamespace defaults to the Pod namespace"

cat >"$TMP_DIR/override-namespace.yaml" <<'EOF'
config:
  kubeNamespace: watched-namespace
EOF
helm template namespace-test "$CHART" --namespace current-namespace \
  --values "$TMP_DIR/override-namespace.yaml" >"$TMP_DIR/override-namespace.out"
override_namespace=$(yq 'select(.kind == "Deployment") | .spec.template.spec.containers[0].env[] | select(.name == "KUBE_NAMESPACE") | .value' "$TMP_DIR/override-namespace.out")
[ "$override_namespace" = "watched-namespace" ] || {
  echo "error: expected KUBE_NAMESPACE watched-namespace, got $override_namespace" >&2
  exit 1
}
echo "PASS kubeNamespace can explicitly override the Pod namespace"

namespace_env() {
  yq 'select(.kind == "Deployment") | .spec.template.spec.containers[0].env[] | select(.name == "KUBE_NAMESPACE") | .value' "$1"
}

cat >"$TMP_DIR/namespace-string.yaml" <<'EOF'
config:
  kubeNamespace: "storage-api-dev,usdsearch"
EOF
helm template namespace-test "$CHART" --namespace current-namespace \
  --values "$TMP_DIR/namespace-string.yaml" >"$TMP_DIR/namespace-string.out"
string_namespaces=$(namespace_env "$TMP_DIR/namespace-string.out")
[ "$string_namespaces" = "storage-api-dev,usdsearch" ] || {
  echo "error: expected KUBE_NAMESPACE storage-api-dev,usdsearch, got $string_namespaces" >&2
  exit 1
}
echo "PASS kubeNamespace accepts a comma-separated string"

cat >"$TMP_DIR/namespace-list.yaml" <<'EOF'
config:
  kubeNamespace:
    - storage-api-dev
    - usdsearch
EOF
helm template namespace-test "$CHART" --namespace current-namespace \
  --values "$TMP_DIR/namespace-list.yaml" >"$TMP_DIR/namespace-list.out"
list_namespaces=$(namespace_env "$TMP_DIR/namespace-list.out")
[ "$list_namespaces" = "storage-api-dev,usdsearch" ] || {
  echo "error: expected KUBE_NAMESPACE storage-api-dev,usdsearch from a list, got $list_namespaces" >&2
  exit 1
}
echo "PASS kubeNamespace accepts a YAML list, joined in order"

cat >"$TMP_DIR/namespace-empty-list.yaml" <<'EOF'
config:
  kubeNamespace: []
EOF
helm template namespace-test "$CHART" --namespace current-namespace \
  --values "$TMP_DIR/namespace-empty-list.yaml" >"$TMP_DIR/namespace-empty-list.out"
empty_list_env=$(yq 'select(.kind == "Deployment") | .spec.template.spec.containers[0].env[] | select(.name == "KUBE_NAMESPACE")' "$TMP_DIR/namespace-empty-list.out")
printf '%s\n' "$empty_list_env" | grep -q 'fieldPath: metadata.namespace'
echo "PASS an empty kubeNamespace list falls back to the Pod namespace"

cat >"$TMP_DIR/default-fqdn.yaml" <<'EOF'
httpProxy:
  enabled: true
  fqdn:
    domain: api.example.com
EOF
helm template fqdn-test "$CHART" --namespace discovery-routing-test \
  --values "$TMP_DIR/default-fqdn.yaml" >"$TMP_DIR/default-fqdn.out"
default_fqdn=$(yq 'select(.kind == "HTTPProxy") | .spec.virtualhost.fqdn' "$TMP_DIR/default-fqdn.out")
[ "$default_fqdn" = "api.example.com" ] || {
  echo "error: expected default HTTPProxy FQDN api.example.com, got $default_fqdn" >&2
  exit 1
}
echo "PASS httpProxy uses fqdn.domain directly by default"

cat >"$TMP_DIR/prefixed-fqdn.yaml" <<'EOF'
httpProxy:
  enabled: true
  fqdn:
    domain: example.com
    host: discovery
EOF
helm template fqdn-test "$CHART" --namespace discovery-routing-test \
  --values "$TMP_DIR/prefixed-fqdn.yaml" >"$TMP_DIR/prefixed-fqdn.out"
prefixed_fqdn=$(yq 'select(.kind == "HTTPProxy") | .spec.virtualhost.fqdn' "$TMP_DIR/prefixed-fqdn.out")
[ "$prefixed_fqdn" = "discovery.example.com" ] || {
  echo "error: expected overridden HTTPProxy FQDN discovery.example.com, got $prefixed_fqdn" >&2
  exit 1
}
echo "PASS httpProxy host prefix remains available as an override"

expect_failure() {
  name=$1
  expected=$2
  values_file=$3
  if helm template routing-test "$CHART" --namespace discovery-routing-test \
      --values "$values_file" >"$TMP_DIR/$name.out" 2>"$TMP_DIR/$name.err"; then
    echo "error: $name unexpectedly rendered" >&2
    exit 1
  fi
  grep -q "$expected" "$TMP_DIR/$name.err" || {
    cat "$TMP_DIR/$name.err" >&2
    echo "error: $name did not report expected validation" >&2
    exit 1
  }
  echo "PASS rejected $name"
}

cat >"$TMP_DIR/no-match.yaml" <<'EOF'
httpProxy:
  enabled: true
  endpoints:
    - name: external
      fqdn: discovery.local
      port: 8081
      additionalRoutes:
        - service: {name: legacy, port: 8080}
EOF
expect_failure no-match 'must set exactly one' "$TMP_DIR/no-match.yaml"

cat >"$TMP_DIR/multiple-match.yaml" <<'EOF'
httpProxy:
  enabled: true
  endpoints:
    - name: external
      fqdn: discovery.local
      port: 8081
      additionalRoutes:
        - exact: /api/v1/services
          prefix: /api/v1
          service: {name: legacy, port: 8080}
EOF
expect_failure multiple-match 'must set exactly one' "$TMP_DIR/multiple-match.yaml"

cat >"$TMP_DIR/no-service-name.yaml" <<'EOF'
httpProxy:
  enabled: true
  endpoints:
    - name: external
      fqdn: discovery.local
      port: 8081
      additionalRoutes:
        - exact: /api/v1/services
          service: {port: 8080}
EOF
expect_failure no-service-name 'service.name is required' "$TMP_DIR/no-service-name.yaml"

cat >"$TMP_DIR/no-service-port.yaml" <<'EOF'
httpProxy:
  enabled: true
  endpoints:
    - name: external
      fqdn: discovery.local
      port: 8081
      additionalRoutes:
        - exact: /api/v1/services
          service: {name: legacy}
EOF
expect_failure no-service-port 'service.port is required' "$TMP_DIR/no-service-port.yaml"

cat >"$TMP_DIR/root-redirect.yaml" <<'EOF'
httpProxy:
  enabled: true
  path: /
  endpoints:
    - name: external
      fqdn: discovery.local
      port: 8081
      additionalRoutes:
        - exact: /api/v1/services
          service: {name: auto-discovery-service, port: 8080}
  rootRedirect:
    enabled: true
    target:
      fqdn: navigator.local
      path: /
      statusCode: 302
EOF

helm template routing-test "$CHART" --namespace discovery-routing-test \
  --values "$TMP_DIR/root-redirect.yaml" >"$TMP_DIR/root-redirect.out"

redirect_routes=$(yq 'select(.kind == "HTTPProxy") | .spec.routes' "$TMP_DIR/root-redirect.out")
printf '%s\n' "$redirect_routes" | grep -q 'exact: "/"'
if printf '%s\n' "$redirect_routes" | grep -q 'requestRedirectPolicy'; then
  echo "error: rootRedirect must not use Contour requestRedirectPolicy" >&2
  exit 1
fi
# Exact `/` service route must disable auth and target the operator.
root_route=$(yq 'select(.kind == "HTTPProxy") | .spec.routes[] | select(.conditions[]?.exact == "/")' "$TMP_DIR/root-redirect.out")
printf '%s\n' "$root_route" | grep -q 'disabled: true'
printf '%s\n' "$root_route" | grep -q 'name: auto-discovery-service'
redirect_line=$(printf '%s\n' "$redirect_routes" | grep -n 'exact: "/"' | head -1 | cut -d: -f1)
catch_all_line=$(printf '%s\n' "$redirect_routes" | grep -n 'prefix: /$' | cut -d: -f1)
[ "$redirect_line" -lt "$catch_all_line" ] || {
  echo "error: rootRedirect must precede catch-all" >&2
  exit 1
}

deploy_env=$(yq 'select(.kind == "Deployment") | .spec.template.spec.containers[0].env' "$TMP_DIR/root-redirect.out")
printf '%s\n' "$deploy_env" | grep -q 'name: ROOT_REDIRECT_URL'
printf '%s\n' "$deploy_env" | grep -q 'http://navigator.local/'
printf '%s\n' "$deploy_env" | grep -q 'name: ROOT_REDIRECT_STATUS'
echo "PASS rootRedirect proxies / to operator with auth disabled"

cat >"$TMP_DIR/root-redirect-missing-fqdn.yaml" <<'EOF'
httpProxy:
  enabled: true
  endpoints:
    - name: external
      fqdn: discovery.local
      port: 8081
  rootRedirect:
    enabled: true
    target:
      path: /
      statusCode: 302
EOF
expect_failure root-redirect-missing-fqdn 'rootRedirect.target.fqdn is required' "$TMP_DIR/root-redirect-missing-fqdn.yaml"
