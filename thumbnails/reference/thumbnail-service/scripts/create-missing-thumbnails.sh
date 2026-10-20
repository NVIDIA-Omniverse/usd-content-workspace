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
# Backfill thumbnails for source files that do not have every configured size.
#
# Ported out of `thumbnail-task-queue/scripts/create-missing-thumbnails.sh`
# before that repository is deleted. The original walked storage, read raw
# metadata keys looking for a `thumbnail_256x256_{FORMAT}_{uuid}` pattern, and
# POSTed a hand-built batch payload to the Python queue's `/image-to-image`.
# All three of those steps are now someone else's job:
#
#   - Whether a source has thumbnails is answered by thumbnail-service's
#     `GET /v1alpha/thumbnails`, so this no longer duplicates the metadata key
#     format. That regex was already drifting — it hard-coded 256x256 and a
#     closed list of encodings, so a deployment configured for any other size
#     saw every file as missing.
#   - Dimensions, encoding, transform, and quality come from the service's
#     `THUMBNAIL_SIZES` and `THUMBNAIL_*` configuration. The old payload pinned
#     them in the script, which is how a backfill could quietly produce
#     thumbnails unlike the ones the pipeline generates.
#   - Dispatch is `POST /v1alpha/thumbnails`, which resolves the identity by
#     Stat and fans out one `omni.thumbnail.generate` event per configured size.
#
# So this is deliberately thinner than what it replaces: enumerate, ask, and
# dispatch. It is a recovery tool, not a pipeline.
#
# The case it exists for: CREATEs that were acknowledged after a successful
# enqueue but never processed — most obviously work still sitting in Celery at
# the cutover, which is abandoned when the Python workers go away.
#
# Usage:
#   scripts/create-missing-thumbnails.sh \
#     --folder https://storage.example/container/projects/ \
#     --token "$(scripts-notifications/get_jwt_token.sh)" \
#     [--service http://localhost:3000] \
#     [--storage localhost:50001] \
#     [--sizes 64x64,128x128,256x256] \
#     [--dry-run] [--limit N]
#
# Requires: buf, grpcurl, curl, jq.

set -euo pipefail

FOLDER=""
TOKEN=""
SERVICE_ENDPOINT="${THUMBNAIL_SERVICE_HTTP_ENDPOINT:-http://localhost:3000}"
STORAGE_ENDPOINT="${STORAGE_GRPC_ENDPOINT:-localhost:50001}"
SIZES="${THUMBNAIL_SIZES:-}"
STORAGE_MODULE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)/storage/api/proto"
DRY_RUN=false
LIMIT=0

usage() {
    sed -n '3,38p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
    exit "${1:-0}"
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --folder)   FOLDER="$2"; shift 2 ;;
        --token)    TOKEN="$2"; shift 2 ;;
        --service)  SERVICE_ENDPOINT="$2"; shift 2 ;;
        --storage)  STORAGE_ENDPOINT="$2"; shift 2 ;;
        --sizes)    SIZES="$2"; shift 2 ;;
        --limit)    LIMIT="$2"; shift 2 ;;
        --dry-run)  DRY_RUN=true; shift ;;
        -h|--help)  usage 0 ;;
        *)          echo "unknown argument: $1" >&2; usage 1 ;;
    esac
done

[[ -n "$FOLDER" ]] || { echo "--folder is required" >&2; usage 1; }
[[ -n "$TOKEN"  ]] || { echo "--token is required" >&2; usage 1; }
[[ -n "$SIZES"  ]] || { echo "--sizes or THUMBNAIL_SIZES is required" >&2; usage 1; }

IFS=',' read -r -a EXPECTED_SIZES <<< "$SIZES"
for index in "${!EXPECTED_SIZES[@]}"; do
    size="${EXPECTED_SIZES[$index]//[[:space:]]/}"
    [[ "$size" =~ ^[1-9][0-9]*x[1-9][0-9]*$ ]] || {
        echo "invalid thumbnail size '${EXPECTED_SIZES[$index]}': expected WIDTHxHEIGHT" >&2
        exit 1
    }
    EXPECTED_SIZES[$index]="$size"
done

for tool in buf grpcurl curl jq; do
    command -v "$tool" >/dev/null || { echo "$tool is required but not installed" >&2; exit 1; }
done

# The storage schema is resolved from the same buf module the proto crates are
# generated from, rather than a vendored copy that can silently fall behind the
# service being called. The storage service serves no reflection, so a
# descriptor is still required.
STORAGE_PROTOSET="$(mktemp -t storage-schema-XXXXXX.binpb)"
trap 'rm -f "${STORAGE_PROTOSET}"' EXIT
buf build "${STORAGE_MODULE}" --as-file-descriptor-set -o "${STORAGE_PROTOSET}"

echo "Backfill scan"
echo "  folder:  $FOLDER"
echo "  storage: $STORAGE_ENDPOINT"
echo "  service: $SERVICE_ENDPOINT"
echo "  sizes:   ${EXPECTED_SIZES[*]}"
$DRY_RUN && echo "  mode:    DRY RUN (nothing will be dispatched)"
echo

# --- 1. Enumerate the folder ------------------------------------------------
#
# ListStat streams, and grpcurl emits one JSON document per response message,
# so `jq -c` over the whole stream flattens it into one address per line.
mapfile -t ADDRESSES < <(
    grpcurl -plaintext \
        -H "authorization: Bearer ${TOKEN}" \
        -protoset "${STORAGE_PROTOSET}" \
        -d "$(jq -nc --arg uri "$FOLDER" '{folder: {uri: $uri}}')" \
        "${STORAGE_ENDPOINT}" \
        nvidia.omniverse.storage.filefolder.v1alpha.FileFolderService/ListStat \
    | jq -r '.entries[]? | select(.resourceAddress != null) | .resourceAddress'
)

if [[ ${#ADDRESSES[@]} -eq 0 ]]; then
    echo "No files found under ${FOLDER}"
    exit 0
fi
echo "Found ${#ADDRESSES[@]} file(s)"

# --- 2. Ask the service which of them already have thumbnails ---------------
#
# One request per source. A backfill is a recovery operation run rarely and
# deliberately, so clarity beats concurrency here; if this ever needs to be
# fast, batch it at the service rather than parallelising the client.
MISSING=()
CHECK_FAILED=0

for address in "${ADDRESSES[@]}"; do
    response_body=$(mktemp "${TMPDIR:-/tmp}/backfill-check.XXXXXX")
    response=$(curl -sS -o "$response_body" -w '%{http_code}' -G \
        --data-urlencode "association_target_resource_address=${address}" \
        -H "Authorization: Bearer ${TOKEN}" \
        "${SERVICE_ENDPOINT}/v1alpha/thumbnails" || echo "000")

    case "$response" in
        200)
            if ! jq -e '.thumbnails | type == "array"' "$response_body" >/dev/null; then
                echo "  ! ${address}: malformed thumbnail response; skipping" >&2
                CHECK_FAILED=$((CHECK_FAILED + 1))
            else
                complete=true
                for size in "${EXPECTED_SIZES[@]}"; do
                    width="${size%x*}"
                    height="${size#*x}"
                    if ! jq -e --argjson width "$width" --argjson height "$height" '
                        any(.thumbnails[]?.specification?;
                            .type == "IMAGE_TO_IMAGE" and
                            .width == $width and .height == $height)
                    ' "$response_body" >/dev/null; then
                        complete=false
                        break
                    fi
                done
                $complete || MISSING+=("$address")
            fi
            ;;
        404) MISSING+=("$address") ;; # none associated
        *)
            # Do not guess. Treating an unknown status as "missing" would
            # re-dispatch work for sources that are fine, which on a large
            # folder is a self-inflicted load spike.
            echo "  ! ${address}: unexpected status ${response}; skipping" >&2
            CHECK_FAILED=$((CHECK_FAILED + 1))
            ;;
    esac
    rm -f "$response_body"
done

echo "  ${#MISSING[@]} missing, $(( ${#ADDRESSES[@]} - ${#MISSING[@]} - CHECK_FAILED )) already present, ${CHECK_FAILED} unchecked"

if [[ ${#MISSING[@]} -eq 0 ]]; then
    echo "Nothing to backfill."
    exit 0
fi

if [[ "$LIMIT" -gt 0 && ${#MISSING[@]} -gt "$LIMIT" ]]; then
    echo "  limiting this run to the first ${LIMIT}"
    MISSING=("${MISSING[@]:0:$LIMIT}")
fi

# --- 3. Dispatch ------------------------------------------------------------
#
# No specification is sent, so the service uses its configured THUMBNAIL_SIZES
# and THUMBNAIL_* defaults — the same ones the storage-CREATE path uses. That
# is the point: a backfilled thumbnail should be indistinguishable from a
# normally generated one.
DISPATCHED=0
FAILED=0

for address in "${MISSING[@]}"; do
    if $DRY_RUN; then
        echo "  would dispatch: ${address}"
        DISPATCHED=$((DISPATCHED + 1))
        continue
    fi

    # mktemp rather than a $$-suffixed name: /tmp is world-writable, and a
    # predictable path is one another process can pre-create as a symlink.
    response=$(mktemp "${TMPDIR:-/tmp}/backfill-response.XXXXXX")

    # Body built by jq, so an address containing a quote or a backslash stays
    # valid JSON instead of producing a request the service rejects.
    status=$(curl -sS -o "$response" -w '%{http_code}' \
        -X POST \
        -H "Authorization: Bearer ${TOKEN}" \
        -H "Content-Type: application/json" \
        -d "$(jq -nc --arg address "$address" '{resource_address: $address}')" \
        "${SERVICE_ENDPOINT}/v1alpha/thumbnails" || echo "000")

    if [[ "$status" == "202" ]]; then
        # 202 with a populated `failed` list is a partial success: some
        # specifications dispatched and some did not. Surface it rather than
        # counting the whole source as done.
        partial=$(jq -r '.failed // [] | length' < "$response" 2>/dev/null || echo 0)
        if [[ "$partial" -gt 0 ]]; then
            echo "  ~ ${address}: ${partial} specification(s) failed to dispatch" >&2
            FAILED=$((FAILED + 1))
        else
            DISPATCHED=$((DISPATCHED + 1))
        fi
    else
        echo "  ! ${address}: HTTP ${status} $(head -c 200 "$response" 2>/dev/null)" >&2
        FAILED=$((FAILED + 1))
    fi
    rm -f "$response"
done

echo
echo "Dispatched: ${DISPATCHED}   Failed: ${FAILED}   Unchecked: ${CHECK_FAILED}"
echo
echo "Dispatch only means the generate events were published. Watch the worker's"
echo "outcome counters for what actually converted:"
echo "  make worker-logs"
echo "  make worker-metrics"

# A backfill that could not dispatch everything should not look like a success
# to whatever ran it.
[[ "$FAILED" -eq 0 && "$CHECK_FAILED" -eq 0 ]]
