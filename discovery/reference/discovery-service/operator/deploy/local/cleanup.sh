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
SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)

if ! kubectl config get-contexts "$CONTEXT" --no-headers 2>/dev/null | grep -q .; then
  echo "error: kubectl context '$CONTEXT' does not exist" >&2
  exit 1
fi

kubectl --context "$CONTEXT" delete namespace "$NAMESPACE" --ignore-not-found --wait=true
if [ "${REMOVE_CONTOUR:-false}" = "true" ]; then
  kubectl --context "$CONTEXT" delete -f "$SCRIPT_DIR/contour-v1.33.5.yaml" --ignore-not-found
fi
