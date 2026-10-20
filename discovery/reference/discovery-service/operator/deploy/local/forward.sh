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
CONTOUR_NAMESPACE=${CONTOUR_NAMESPACE:-projectcontour}
LOCAL_PORT=${LOCAL_PORT:-18080}

if ! command -v kubectl >/dev/null 2>&1; then
  echo "error: kubectl is required" >&2
  exit 1
fi

if ! kubectl config get-contexts "$CONTEXT" --no-headers 2>/dev/null | grep -q .; then
  echo "error: kubectl context '$CONTEXT' does not exist" >&2
  exit 1
fi

cat <<EOF
Forwarding Contour Envoy to http://127.0.0.1:$LOCAL_PORT
Keep this command running, then use another terminal:

  curl --resolve discovery.local:$LOCAL_PORT:127.0.0.1 http://discovery.local:$LOCAL_PORT/api/v1/services
  curl --resolve discovery.local:$LOCAL_PORT:127.0.0.1 http://discovery.local:$LOCAL_PORT/api/v1alpha/services
  curl --resolve discovery.local:$LOCAL_PORT:127.0.0.1 http://discovery.local:$LOCAL_PORT/api/v1/auth-config
  curl --resolve discovery.local:$LOCAL_PORT:127.0.0.1 http://discovery.local:$LOCAL_PORT/api/v1alpha/auth-config
  curl --resolve discovery.local:$LOCAL_PORT:127.0.0.1 http://discovery.local:$LOCAL_PORT/api/v2/auth-config
  curl --resolve discovery.local:$LOCAL_PORT:127.0.0.1 http://discovery.local:$LOCAL_PORT/api/v2beta/auth-config
  curl --resolve discovery.local:$LOCAL_PORT:127.0.0.1 http://discovery.local:$LOCAL_PORT/api/v2beta/services
  curl --resolve discovery.local:$LOCAL_PORT:127.0.0.1 http://discovery.local:$LOCAL_PORT/api/v2/services

Press Ctrl-C to stop forwarding.
EOF

exec kubectl --context "$CONTEXT" -n "$CONTOUR_NAMESPACE" \
  port-forward service/envoy "$LOCAL_PORT:80"
