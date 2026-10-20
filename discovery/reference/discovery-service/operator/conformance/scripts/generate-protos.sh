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
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
UV="${UV:-uv}"
OUT="$ROOT/conformance/discovery_conformance/generated"
PROTO="$ROOT/discovery-protos/proto"
mkdir -p "$OUT"
"$UV" run --project "$ROOT/conformance" --locked --extra generate python -m grpc_tools.protoc --proto_path="$PROTO" --python_out="$OUT" --grpc_python_out="$OUT" \
  "$PROTO/nvidia/omniverse/discovery/capabilities/v2beta/capabilities.proto" \
  "$PROTO/nvidia/omniverse/discovery/capabilities/v2beta/capabilities_service.proto"
# Keep generated imports isolated from any installed top-level nvidia package.
"$UV" run --project "$ROOT/conformance" --locked --extra generate python - "$OUT" <<'PY'
from pathlib import Path
import sys

root = Path(sys.argv[1])
for path in root.rglob('*_pb2*.py'):
    text = path.read_text()
    text = text.replace('from nvidia.omniverse.discovery.capabilities.v2beta import ', 'from . import ')
    path.write_text(text)
for directory in [root, *(p for p in root.rglob('*') if p.is_dir())]:
    (directory / '__init__.py').touch()
PY
