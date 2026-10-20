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

set -euo pipefail

compose_file="test/docker-compose-testing.yaml"
network="envoy-auth-extension-testing_envoy-auth-test-network"
server="envoy-auth-ext-service-test:50051"
grpcurl_image="fullstorydev/grpcurl:v1.9.3@sha256:085e183ca334eb4e81ca81ee12cbb2b2737505d1d77f5e33dabc5d066593d998"

cleanup() {
    docker compose -f "$compose_file" down --remove-orphans
}
trap cleanup EXIT

rpc() {
    local method="$1"
    shift
    docker run --rm --network "$network" "$grpcurl_image" \
        -max-time 10 -plaintext "$@" "$server" "$method"
}

docker compose -f "$compose_file" build
docker compose -f "$compose_file" up -d --no-build --wait --wait-timeout 30

health_response="$(rpc grpc.health.v1.Health/Check \
    -d '{"service":"envoy.service.auth.v3.Authorization"}')"
grep -q '"status": "SERVING"' <<<"$health_response"

services="$(rpc list)"
grep -qx 'envoy.service.auth.v3.Authorization' <<<"$services"
grep -qx 'grpc.health.v1.Health' <<<"$services"
grep -qx 'grpc.reflection.v1alpha.ServerReflection' <<<"$services"

valid_response="$(rpc envoy.service.auth.v3.Authorization/Check \
    -d '{"attributes":{"contextExtensions":{},"request":{"http":{"headers":{"authorization":"Bearer valid-token"}}}}}')"
grep -q '"message": "Authorization Success"' <<<"$valid_response"
grep -q '"okResponse"' <<<"$valid_response"
grep -q '"key": "user-id"' <<<"$valid_response"
grep -q '"value": "integration-user"' <<<"$valid_response"
grep -q '"key": "username"' <<<"$valid_response"

invalid_response="$(rpc envoy.service.auth.v3.Authorization/Check \
    -d '{"attributes":{"contextExtensions":{},"request":{"http":{"headers":{"authorization":"Bearer invalid-token"}}}}}')"
grep -q '"code": 16' <<<"$invalid_response"
grep -q '"code": "Unauthorized"' <<<"$invalid_response"

missing_response="$(rpc envoy.service.auth.v3.Authorization/Check \
    -d '{"attributes":{"contextExtensions":{},"request":{"http":{"headers":{}}}}}')"
grep -q '"code": 16' <<<"$missing_response"
grep -q '"code": "Unauthorized"' <<<"$missing_response"

printf '%s\n' "Integration tests passed"
