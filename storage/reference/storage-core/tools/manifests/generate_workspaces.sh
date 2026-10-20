#!/bin/bash
# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

set -euo pipefail

# Seed the .wkspc/.manifest workspace integration-test fixtures and generate their
# `.manifest` files on the live backends. Companion to the .omnimfst-era
# generate_manifest.sh (kept for trunk); this one targets the overlay model.
#
# Two phases, per backend:
#   1. SEED  — upload every static file under FIXTURES_DIR to <base>/<PREFIX>/<relpath>,
#              idempotently (skip if it already exists — important on versioned buckets
#              so resource identities don't churn).
#   2. GEN   — for each workspace in manifest-spec.json, Stat each entry's source object
#              (always via the storage service), build the standard manifest JSON
#              (relative_path from spec, resource_info from Stat), upload to
#              <base>/<PREFIX>/<workspace>/.manifest.
#
# Stat always goes through the locally-running storage service via grpcurl (reads public
# objects anonymously, private/versioned with creds). WRITES by backend kind:
#   service       (S3 priv/versioned, Azure priv/versioned) — grpcurl Write
#   s3_public     — aws s3 cp --acl public-read
#   manual        (Azure public account) — no account key in env; emit an
#                  az-upload command script for the user to run with that account's key.
#
# Env: OMNI_STORAGE_SERVICE_{S3_BUCKET,S3_PUBLIC_BUCKET,S3_VERSIONED_BUCKET} and
#      OMNI_STORAGE_SERVICE_AZURE_{PRIVATE,PUBLIC,VERSIONED}_ACCOUNT, OMNI_STORAGE_SERVICE_AZURE_PUBLIC_CONTAINER
#      (the test bucket/account names the integration tests read); S3_PORT, AZURE_PORT (from run.sh); FIXTURES_DIR; PREFIX (default spec .prefix);
#      BACKENDS (label filter); DRY_RUN=1 (print, write nothing).

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FIXTURES_DIR="${FIXTURES_DIR:-$SCRIPT_DIR/workspace-fixtures}"
SPEC="$FIXTURES_DIR/manifest-spec.json"

S3_PORT="${S3_PORT:-50051}"
AZURE_PORT="${AZURE_PORT:-50052}"
DRY_RUN="${DRY_RUN:-0}"

[ -f "$SPEC" ] || { echo "spec not found: $SPEC" >&2; exit 1; }
PREFIX="${PREFIX:-$(jq -r '.prefix' "$SPEC")}"
STORAGE_API_VERSION="$(jq -r '.storage_api_version' "$SPEC")"

S3_BUCKET="${OMNI_STORAGE_SERVICE_S3_BUCKET:?set to the private S3 test bucket}"
S3_PUBLIC_BUCKET="${OMNI_STORAGE_SERVICE_S3_PUBLIC_BUCKET:?set to the public S3 test bucket}"
S3_VERSIONED_BUCKET="${OMNI_STORAGE_SERVICE_S3_VERSIONED_BUCKET:?set to the versioned S3 test bucket}"
AZURE_PRIVATE_ACCOUNT="${OMNI_STORAGE_SERVICE_AZURE_PRIVATE_ACCOUNT:?set to the private Azure test account}"
AZURE_PUBLIC_ACCOUNT="${OMNI_STORAGE_SERVICE_AZURE_PUBLIC_ACCOUNT:?set to the public Azure test account}"
AZURE_VERSIONED_ACCOUNT="${OMNI_STORAGE_SERVICE_AZURE_VERSIONED_ACCOUNT:?set to the versioned Azure test account}"
AZURE_PUBLIC_CONTAINER="${OMNI_STORAGE_SERVICE_AZURE_PUBLIC_CONTAINER:?set to the public Azure test container}"
AZURE_PUBLIC_BLOB_PREFIX="storage-service-unittests"

# Persistent outputs for the manual Azure-public step.
PENDING_DIR="$SCRIPT_DIR/pending-public-uploads"
AZ_CMDS="$PENDING_DIR/azure_public_upload.sh"

# label | base_url | grpc_port | kind
BACKEND_ROWS=(
  "s3_private|https://${S3_BUCKET}.s3.us-west-2.amazonaws.com|$S3_PORT|service"
  "s3_versioned|https://${S3_VERSIONED_BUCKET}.s3.us-west-2.amazonaws.com|$S3_PORT|service"
  "azure_private|https://${AZURE_PRIVATE_ACCOUNT}.blob.core.windows.net/unittests/storage-service-unittests|$AZURE_PORT|service"
  "azure_versioned|https://${AZURE_VERSIONED_ACCOUNT}.blob.core.windows.net/unittests/storage-service-unittests|$AZURE_PORT|service"
  "s3_public|https://${S3_PUBLIC_BUCKET}.s3.us-west-2.amazonaws.com|$S3_PORT|s3_public"
  "azure_public|https://${AZURE_PUBLIC_ACCOUNT}.blob.core.windows.net/${AZURE_PUBLIC_CONTAINER}/storage-service-unittests|$AZURE_PORT|manual"
)

BACKEND_FILTER="${BACKENDS:-}"
want_backend() { [ -z "$BACKEND_FILTER" ] && return 0; case ",$BACKEND_FILTER," in *",$1,"*) return 0 ;; *) return 1 ;; esac; }

FOS="nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService"

stat_identity() {  # port addr -> "<identity>|<size>|<mtime>" or rc 1
  local port="$1" addr="$2" resp identity
  resp="$(grpcurl --plaintext -d "{\"resource_address\": \"${addr}\"}" "localhost:${port}" "${FOS}.Stat" 2>/dev/null)" || return 1
  identity="$(echo "$resp" | jq -r '.resourceInfo.resourceIdentity.encodedIdentity // empty')"
  [ -n "$identity" ] || return 1
  printf '%s|%s|%s' "$identity" \
    "$(echo "$resp" | jq -r '.resourceInfo.metadata.dataObjectSize // 0')" \
    "$(echo "$resp" | jq -r '.resourceInfo.metadata.lastModifiedTimestamp // empty')"
}

service_write() {  # port addr file
  local port="$1" addr="$2" file="$3" size b64
  size="$(wc -c < "$file")"; b64="$(base64 -w 0 "$file")"
  printf '{"params":{"destinationResourceAddress":"%s","dataObjectSize":%d,"uploadPreference":"UPLOAD_PREFERENCE_BODY"}}\n{"chunk":{"chunk":"%s"}}\n' \
    "$addr" "$size" "$b64" | grpcurl --plaintext -d @ "localhost:${port}" "${FOS}.Write" >/dev/null
}

obj_exists() {  # kind base port rel
  case "$1" in
    service) stat_identity "$3" "${2}/${PREFIX}/${4}" >/dev/null 2>&1 ;;
    s3_public)
      AWS_ACCESS_KEY_ID="$OMNI_STORAGE_SERVICE_AWS_ACCESS_KEY_ID" \
      AWS_SECRET_ACCESS_KEY="$OMNI_STORAGE_SERVICE_AWS_SECRET_ACCESS_KEY" \
        aws s3api head-object --region us-west-2 --bucket "$S3_PUBLIC_BUCKET" --key "${PREFIX}/${4}" >/dev/null 2>&1 ;;
    *) return 1 ;;
  esac
}

obj_put() {  # kind base port rel file
  case "$1" in
    service) service_write "$3" "${2}/${PREFIX}/${4}" "$5" ;;
    s3_public)
      AWS_ACCESS_KEY_ID="$OMNI_STORAGE_SERVICE_AWS_ACCESS_KEY_ID" \
      AWS_SECRET_ACCESS_KEY="$OMNI_STORAGE_SERVICE_AWS_SECRET_ACCESS_KEY" \
        aws s3 cp "$5" "s3://${S3_PUBLIC_BUCKET}/${PREFIX}/${4}" --region us-west-2 --acl public-read >/dev/null ;;
  esac
}

az_manual_cmd() {  # rel file  -> append an az upload command for the public Azure container
  local rel="$1" file="$2" blob="${AZURE_PUBLIC_BLOB_PREFIX}/${PREFIX}/${rel}"
  {
    printf 'az storage blob upload --account-name %s --account-key "$AZURE_PUBLIC_ACCOUNT_KEY" \\\n' "$AZURE_PUBLIC_ACCOUNT"
    printf '  --container-name %s --overwrite \\\n' "$AZURE_PUBLIC_CONTAINER"
    printf '  --name %q --file %q\n' "$blob" "$file"
  } >> "$AZ_CMDS"
}

static_files() { find "$FIXTURES_DIR" -type f ! -name 'README.md' ! -name 'manifest-spec.json' | LC_ALL=C sort; }

seed_backend() {  # kind base port
  local kind="$1" base="$2" port="$3" seeded=0 skipped=0
  echo "  -- seed static files --"
  while IFS= read -r file; do
    local rel="${file#"$FIXTURES_DIR"/}"
    if [ "$kind" = manual ]; then az_manual_cmd "$rel" "$file"; seeded=$((seeded+1)); continue; fi
    if obj_exists "$kind" "$base" "$port" "$rel"; then skipped=$((skipped+1)); continue; fi
    if [ "$DRY_RUN" = 1 ]; then echo "     PUT  ${PREFIX}/${rel}"; else obj_put "$kind" "$base" "$port" "$rel" "$file"; fi
    seeded=$((seeded+1))
  done < <(static_files)
  echo "     $([ "$kind" = manual ] && echo "queued(manual)=$seeded" || echo "seeded=$seeded skipped(existing)=$skipped")"
}

build_manifest_json() {  # ws base port out
  local ws="$1" base="$2" port="$3" out="$4" n first=true
  printf '{\n  "format_version": { "version_string": "1.0" },\n  "storage_api_version": "%s",\n  "files": [\n' "$STORAGE_API_VERSION" > "$out"
  n="$(jq -r --arg ws "$ws" '.workspaces[$ws] | length' "$SPEC")"
  for ((i=0; i<n; i++)); do
    local rel src trip identity size mtime
    rel="$(jq -r --arg ws "$ws" ".workspaces[\$ws][$i].relative_path" "$SPEC")"
    src="$(jq -r --arg ws "$ws" ".workspaces[\$ws][$i].source" "$SPEC")"
    if ! trip="$(stat_identity "$port" "${base}/${PREFIX}/${src}")"; then
      echo "     !! Stat failed for source ${PREFIX}/${src} (skipping ${rel})" >&2; continue
    fi
    identity="${trip%%|*}"; trip="${trip#*|}"; size="${trip%%|*}"; mtime="${trip#*|}"
    [ "$first" = false ] && printf ',\n' >> "$out"; first=false
    if [ -n "$mtime" ]; then
      printf '    { "relative_path": %s, "resource_info": { "resource_identity": %s, "size": %s, "modified_time": %s } }' \
        "$(jq -nc --arg v "$rel" '$v')" "$(jq -nc --arg v "$identity" '$v')" "$size" "$(jq -nc --arg v "$mtime" '$v')" >> "$out"
    else
      printf '    { "relative_path": %s, "resource_info": { "resource_identity": %s, "size": %s } }' \
        "$(jq -nc --arg v "$rel" '$v')" "$(jq -nc --arg v "$identity" '$v')" "$size" >> "$out"
    fi
  done
  printf '\n  ]\n}\n' >> "$out"
}

generate_backend() {  # kind base port label
  local kind="$1" base="$2" port="$3" label="$4"
  echo "  -- generate .manifest files --"
  while IFS= read -r ws; do
    local rel="${ws}/.manifest" out
    if [ "$kind" = manual ]; then out="${PENDING_DIR}/${ws}.manifest"; else out="${WORKDIR}/${label}__${ws}.manifest"; fi
    build_manifest_json "$ws" "$base" "$port" "$out"
    jq empty "$out" 2>/dev/null || { echo "     !! invalid JSON for $ws" >&2; continue; }
    if [ "$kind" = manual ]; then
      # The manifest bakes identities of the sources/* objects, which must already exist on this
      # (public) backend. On a first pass they don't (we can't seed without a key), so Stat yields
      # an empty files[] — don't ship a broken manifest. Tell the user to seed first, then re-run.
      if [ "$(jq '.files | length' "$out")" -eq 0 ]; then
        echo "     !! ${ws}/.manifest has 0 entries — seed the static files to ${AZURE_PUBLIC_ACCOUNT} first," >&2
        echo "        then re-run with BACKENDS=azure_public to (re)generate this manifest." >&2
      else
        az_manual_cmd "$rel" "$out"; echo "     queued(manual) ${PREFIX}/${rel}"
      fi
      continue
    fi
    if [ "$DRY_RUN" = 1 ]; then echo "     PUT  ${PREFIX}/${rel} ($(wc -c <"$out") bytes)";
    else obj_put "$kind" "$base" "$port" "$rel" "$out"; echo "     wrote ${PREFIX}/${rel}"; fi
  done < <(jq -r '.workspaces | keys[]' "$SPEC")
}

WORKDIR="$(mktemp -d)"; trap 'rm -rf "$WORKDIR"' EXIT
mkdir -p "$PENDING_DIR"; : > "$AZ_CMDS"
{ echo "#!/bin/bash"; echo "set -euo pipefail"; echo "# Azure public account uploads — no account key in the standard env."; echo "# Set AZURE_PUBLIC_ACCOUNT_KEY to the public Azure test account key, then run this script."; echo ": \"\${AZURE_PUBLIC_ACCOUNT_KEY:?set AZURE_PUBLIC_ACCOUNT_KEY to the public Azure test account key}\""; echo ""; } >> "$AZ_CMDS"

echo "Prefix: $PREFIX   Fixtures: $FIXTURES_DIR   DRY_RUN=$DRY_RUN"
for row in "${BACKEND_ROWS[@]}"; do
  IFS='|' read -r label base port kind <<< "$row"
  want_backend "$label" || continue
  echo "=== $label  ($base)  [$kind] ==="
  seed_backend "$kind" "$base" "$port"
  generate_backend "$kind" "$base" "$port" "$label"
  echo ""
done

if [ "$(wc -l < "$AZ_CMDS")" -gt 6 ]; then
  chmod +x "$AZ_CMDS"
  echo "==> The public Azure account ($AZURE_PUBLIC_ACCOUNT) has no account key in the environment."
  echo "    Generated an upload script: $AZ_CMDS"
  echo "    Run it with that account's key:  AZURE_PUBLIC_ACCOUNT_KEY=<key> $AZ_CMDS"
fi
echo "Done."
