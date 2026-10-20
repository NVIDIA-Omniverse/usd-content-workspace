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

#
# Regenerate the checked-in protobuf crates under crates/proto/.
#
# Generated code is committed, so `cargo build` and CI need neither protoc nor
# schema-registry access. This script is the deliberate step that updates it, and the
# only supported way to do so: the generator owns each crate's Cargo.toml, lib.rs and
# src/**, so hand-edits there are lost on the next run. The hand-maintained inputs are
# each crate's buf.gen.yaml and Cargo.toml.template.
#
# Prerequisites:
#
#   # buf invokes these by name from PATH; the versions are pinned so the output is
#   # reproducible and a drift check is a diff anyone can run.
#   cargo install protoc-gen-prost@0.5.0 protoc-gen-prost-crate@0.5.0 protoc-gen-tonic@0.5.0
#
#   # discovery, notifications and storage generate from the protos published
#   # alongside this service in the same repository.
#
# Usage: ./scripts/buf-gen.sh [crate ...]      (default: all of them)

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ALL_CRATES=(discovery notifications storage thumbnail)
CRATES=("${@:-${ALL_CRATES[@]}}")

for tool in buf protoc protoc-gen-prost protoc-gen-prost-crate protoc-gen-tonic; do
    command -v "$tool" >/dev/null \
        || { echo "$tool is required but not installed; see the header of $0" >&2; exit 1; }
done

for crate in "${CRATES[@]}"; do
    dir="$ROOT/crates/proto/$crate"
    [[ -f "$dir/buf.gen.yaml" ]] || { echo "no such proto crate: $crate" >&2; exit 1; }

    echo "==> regenerating $crate"
    # The generator owns src/ outright, and it has to start empty: a package that goes
    # away upstream should go away here too, not linger as a stale file that still
    # compiles. The committed tree is only moved aside, though, and restored if
    # generation fails — an expired registry login should not leave the workspace
    # without the sources it needs to build.
    stash="$(mktemp -d)"
    if [[ -d "$dir/src" ]]; then
        mv "$dir/src" "$stash/src"
    fi

    if (cd "$dir" && buf generate); then
        rm -rf "$stash"
    else
        rm -rf "$dir/src"
        if [[ -d "$stash/src" ]]; then
            mv "$stash/src" "$dir/src"
        fi
        rm -rf "$stash"
        echo "buf generate failed for $crate; restored the committed sources" >&2
        exit 1
    fi
done

# Generated sources ship in the open-source release, so they carry the same SPDX
# header as hand-written code. buf has no header option, so add it after generation.
for crate in "${CRATES[@]}"; do
    find "$ROOT/crates/proto/$crate/src" -name '*.rs' -print0 |
        while IFS= read -r -d '' file; do
            grep -q 'SPDX-License-Identifier' "$file" && continue
            # Match the file's line endings so a CRLF file does not end up mixed.
            if grep -q $'\r$' "$file"; then eol='s/$/\r/'; else eol=''; fi
            { sed -e "$eol" <<'HEADER'
// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

HEADER
              cat "$file"; } > "$file.tmp" && mv "$file.tmp" "$file"
        done
done

# rustfmt rewraps the generated lib.rs and `cargo fmt --check` is a hard CI gate, so
# formatting is part of regenerating rather than something to remember afterwards.
(cd "$ROOT" && cargo fmt --all)

cat <<EOF

Regenerated: ${CRATES[*]}

Review the diff — it should contain only generator output — then run:
  cargo test --workspace
  cargo clippy --all-targets --all-features -- -D warnings
EOF
