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

# Advisory check: flag configuration the code reads and `.env.example` does not mention.
#
# README.md and CLAUDE.md point at `.env.example` as the list of what this service
# reads. It drifted badly enough that `THUMBNAIL_SERVICE_GRPC_ENDPOINT` -- required,
# no default, a hard startup failure for task-worker -- was absent, which costs
# whoever hits it an afternoon. The variables are declared in one recognizable
# shape, so the drift is checkable rather than a matter of remembering.
#
# Advisory, never blocking, for the same reason `check-spec-delta.sh` is: this is a
# grep over `env = "..."` attributes, so it is wrong in both directions. A variable
# read through a helper or by a dependency does not appear, and a variable
# documented for an operator that no Rust code reads looks like an extra. A
# blocking gate on a heuristic gets satisfied with placeholder entries, which is
# worse than no check -- documentation that looks present and says nothing.
#
# What counts as documented, on the `.env.example` side: a line matching
# `^[[:space:]]*#?[[:space:]]*NAME=`. So `NAME=value` and the commented
# `# NAME=value` both count, and a mention in prose does not -- naming a variable
# in a sentence tells a reader it exists but not what to set, and it is exactly
# the shape a placeholder-satisfied gate would produce. `ENABLE_ACCESS_TOKEN_FILE`
# was the case that made this worth stating: mentioned twice in prose, absent as
# an entry, and reported missing for that reason.
#
# The mirror of the `env = "..."` false negative above: this side is wrong when a
# variable is deliberately not offered locally. Document it in the commented form
# with the inert value -- `# ENABLE_ACCESS_TOKEN_FILE=false` -- so the entry
# exists without inviting a local override that the surrounding prose forbids.
#
# Usage: check-env-drift.sh
#   Exits 0 always. Prints the two-way difference.

set -uo pipefail

cd "$(dirname "$0")/.." || exit 0

ENV_FILE=".env.example"
CONFIG_SOURCES=(
    crates/common/src/config.rs
    crates/task-worker/src/config.rs
)

if [[ ! -f "$ENV_FILE" ]]; then
    echo "check-env-drift: no $ENV_FILE; nothing to compare."
    exit 0
fi

# Declared by clap: `env = "NAME"`.
declared_from_attributes() {
    grep -ho 'env = "[A-Z0-9_]*"' "${CONFIG_SOURCES[@]}" 2>/dev/null |
        sed 's/env = "//; s/"//'
}

# Declared by `outbound_auth_env_keys()`, which lists the names the shared auth
# args consume. Those live in an external crate, so no `env = "..."` attribute
# exists here for them to be found by.
declared_from_auth_keys() {
    sed -n '/fn outbound_auth_env_keys/,/^}/p' crates/common/src/config.rs |
        grep -o '"[A-Z0-9_]*"' | tr -d '"'
}

# Read at runtime or by a dependency, so they carry no `env = "..."` attribute
# here. Listed by hand because the alternative is a grep for `env::var` that finds
# the test fixtures too.
#
#   RUST_LOG           read by `logging::init_tracing` when building the filter.
#   OTEL_SERVICE_NAME  read by otel-tools' resource detector. Note it is not
#                      SERVICE_NAME: that name appears in the chart and in CI but
#                      no code reads it, and `init_tracing` passes a hardcoded
#                      literal for the service name regardless.
runtime_reads() {
    printf '%s\n' RUST_LOG OTEL_SERVICE_NAME
}

declared=$(
    {
        declared_from_attributes
        declared_from_auth_keys
        runtime_reads
    } | sort -u
)

# Anything in a `KEY=` position counts as documented, commented out or not: a
# commented example is documentation, and requiring it to be active would push
# operators toward uncommenting settings they do not need.
documented=$(
    grep -hoE '^[[:space:]]*#?[[:space:]]*[A-Z][A-Z0-9_]*=' "$ENV_FILE" |
        sed 's/^[[:space:]]*#\?[[:space:]]*//; s/=$//' | sort -u
)

missing=$(comm -23 <(echo "$declared") <(echo "$documented"))
undocumented_count=$(echo "$missing" | grep -c '[^[:space:]]')

extra=$(comm -13 <(echo "$declared") <(echo "$documented"))
extra_count=$(echo "$extra" | grep -c '[^[:space:]]')

echo "check-env-drift: $(echo "$declared" | grep -c '[^[:space:]]') declared, $(echo "$documented" | grep -c '[^[:space:]]') documented."

status=0
if ((undocumented_count > 0)); then
    echo
    echo "Read by the code, missing from $ENV_FILE ($undocumented_count):"
    echo "$missing" | sed 's/^/  /'
    status=1
fi

if ((extra_count > 0)); then
    echo
    echo "In $ENV_FILE, declared by no config module ($extra_count):"
    echo "$extra" | sed 's/^/  /'
    echo "  (Not necessarily wrong: an operator-facing variable consumed by a"
    echo "   dependency, or read at runtime, belongs here and has no attribute.)"
    status=1
fi

if ((status == 0)); then
    echo "No drift."
else
    echo
    echo "Advisory only; this never fails a pipeline."
fi

exit 0
