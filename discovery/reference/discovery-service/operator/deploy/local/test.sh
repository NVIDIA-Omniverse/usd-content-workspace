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
LOCAL_PORT=${LOCAL_PORT:-18080}
HOST=${HOST:-discovery.local}
PF_LOG=${TMPDIR:-/tmp}/discovery-routing-port-forward.log
PF_PID=""

# shellcheck disable=SC2329 # Invoked through trap.
cleanup() {
  if [ -n "$PF_PID" ]; then
    kill "$PF_PID" 2>/dev/null || true
    wait "$PF_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT HUP INT TERM

for command in kubectl curl jq; do
  command -v "$command" >/dev/null 2>&1 || { echo "error: $command is required" >&2; exit 1; }
done

if ! kubectl config get-contexts "$CONTEXT" --no-headers 2>/dev/null | grep -q .; then
  echo "error: kubectl context '$CONTEXT' does not exist" >&2
  exit 1
fi

proxy_name=auto-discovery-service-external
attempt=0
while [ "$attempt" -lt 60 ]; do
  valid=$(kubectl --context "$CONTEXT" -n "$NAMESPACE" get httpproxy "$proxy_name" \
    -o jsonpath='{.status.conditions[?(@.type=="Valid")].status}' 2>/dev/null || true)
  [ "$valid" = "True" ] && break
  attempt=$((attempt + 1))
  sleep 2
done
if [ "${valid:-}" != "True" ]; then
  kubectl --context "$CONTEXT" -n "$NAMESPACE" describe httpproxy "$proxy_name" >&2 || true
  echo "error: HTTPProxy did not become valid" >&2
  exit 1
fi

kubectl --context "$CONTEXT" -n "$CONTOUR_NAMESPACE" port-forward service/envoy "$LOCAL_PORT:80" >"$PF_LOG" 2>&1 &
PF_PID=$!

attempt=0
while [ "$attempt" -lt 30 ]; do
  if curl --silent --fail --output /dev/null -H "Host: $HOST" "http://127.0.0.1:$LOCAL_PORT/api/v1/services"; then
    break
  fi
  if ! kill -0 "$PF_PID" 2>/dev/null; then
    cat "$PF_LOG" >&2
    echo "error: Envoy port-forward exited" >&2
    exit 1
  fi
  attempt=$((attempt + 1))
  sleep 1
done

assert_schema() {
  path=$1
  expected=$2
  body=$(curl --silent --show-error --fail -H "Host: $HOST" "http://127.0.0.1:$LOCAL_PORT$path")
  actual=$(printf '%s' "$body" | jq -er '."schema-version"')
  if [ "$actual" != "$expected" ]; then
    echo "error: $path returned schema-version $actual, expected $expected" >&2
    printf '%s\n' "$body" >&2
    exit 1
  fi
  echo "PASS $path -> schema-version $actual"
}

assert_schema /api/v1/services 1
assert_schema /api/v1alpha/services 1

assert_auth_config() {
  path=$1
  body=$(curl --silent --show-error --fail -H "Host: $HOST" "http://127.0.0.1:$LOCAL_PORT$path")
  if ! printf '%s' "$body" | jq -e \
    '.openid_configuration == "https://identity.example.test/.well-known/openid-configuration" and .clients.default.client_id == "local-routing-test-client"' >/dev/null; then
    echo "error: $path did not return the expected auth config" >&2
    printf '%s\n' "$body" >&2
    exit 1
  fi
  echo "PASS $path -> operator auth config"
}

assert_auth_config /api/v1/auth-config
assert_auth_config /api/v1alpha/auth-config
assert_auth_config /api/v2/auth-config
assert_auth_config /api/v2beta/auth-config

attempt=0
while [ "$attempt" -lt 60 ]; do
  body=$(curl --silent --show-error --fail -H "Host: $HOST" "http://127.0.0.1:$LOCAL_PORT/api/v2beta/services")
  if printf '%s' "$body" | jq -e '."schema-version" == 2 and any(.services[]?; .name == "mock-capabilities" and any(.rest.endpoints[]?; .name == "fileobject"))' >/dev/null; then
    echo "PASS /api/v2beta/services -> schema-version 2 with discovered mock capability"
    alias_body=$(curl --silent --show-error --fail -H "Host: $HOST" "http://127.0.0.1:$LOCAL_PORT/api/v2/services")
    if [ "$(printf '%s' "$body" | jq -S .)" != "$(printf '%s' "$alias_body" | jq -S .)" ]; then
      echo "FAIL /api/v2/services alias response differs from /api/v2beta/services" >&2
      exit 1
    fi
    echo "PASS /api/v2/services -> same response as /api/v2beta/services"
    exit 0
  fi
  attempt=$((attempt + 1))
  sleep 2
done

printf '%s\n' "$body" >&2
echo "error: v2 response did not discover the mock capability" >&2
exit 1
