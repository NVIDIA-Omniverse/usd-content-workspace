#!/bin/bash
# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

set -euo pipefail

# Configuration
STORAGE_API_VERSION="v1alpha"
MANIFEST_SUBPATH="unittests/manifests"

# gRPC ports for each backend's storage service instance
S3_PORT="${S3_PORT:-50051}"
AZURE_PORT="${AZURE_PORT:-50052}"

# Test bucket/account names: the same variables the integration tests read
# (see storage-core-integration-tests/src/names.rs).
: "${OMNI_STORAGE_SERVICE_S3_BUCKET:?set to the private S3 test bucket}"
: "${OMNI_STORAGE_SERVICE_S3_PUBLIC_BUCKET:?set to the public S3 test bucket}"
: "${OMNI_STORAGE_SERVICE_S3_VERSIONED_BUCKET:?set to the versioned S3 test bucket}"
: "${OMNI_STORAGE_SERVICE_AZURE_PRIVATE_ACCOUNT:?set to the private Azure test account}"
: "${OMNI_STORAGE_SERVICE_AZURE_PUBLIC_ACCOUNT:?set to the public Azure test account}"
: "${OMNI_STORAGE_SERVICE_AZURE_VERSIONED_ACCOUNT:?set to the versioned Azure test account}"
: "${OMNI_STORAGE_SERVICE_AZURE_PUBLIC_CONTAINER:?set to the public Azure test container}"
S3_PUBLIC_URL="https://${OMNI_STORAGE_SERVICE_S3_PUBLIC_BUCKET}.s3.us-west-2.amazonaws.com"

# Base URLs from integration tests: S3 (public, private, versioned) + Azure (public, private, versioned)
LABELS=(
    "s3_public"
    "s3_private"
    "s3_versioned"
    "azure_public"
    "azure_private"
    "azure_versioned"
)
URLS=(
    "$S3_PUBLIC_URL"
    "https://${OMNI_STORAGE_SERVICE_S3_BUCKET}.s3.us-west-2.amazonaws.com"
    "https://${OMNI_STORAGE_SERVICE_S3_VERSIONED_BUCKET}.s3.us-west-2.amazonaws.com"
    "https://${OMNI_STORAGE_SERVICE_AZURE_PUBLIC_ACCOUNT}.blob.core.windows.net/${OMNI_STORAGE_SERVICE_AZURE_PUBLIC_CONTAINER}/storage-service-unittests"
    "https://${OMNI_STORAGE_SERVICE_AZURE_PRIVATE_ACCOUNT}.blob.core.windows.net/unittests/storage-service-unittests"
    "https://${OMNI_STORAGE_SERVICE_AZURE_VERSIONED_ACCOUNT}.blob.core.windows.net/unittests/storage-service-unittests"
)
PORTS=(
    "$S3_PORT"
    "$S3_PORT"
    "$S3_PORT"
    "$AZURE_PORT"
    "$AZURE_PORT"
    "$AZURE_PORT"
)
# Whether the backend is writable (public buckets/containers are read-only)
WRITABLE=(
    false
    true
    true
    false
    true
    true
)

# Array of relative paths
RELATIVE_PATHS=(
    "folder_b/jellyfish - hevc.mkv"
    "folder_a/hello.txt"
    "folder_a/goodbye.txt"
    "folder_a/nested/aye.txt"
    "folder_a/nested/bee.txt"
)

# Determine the manifest version we should generate.
#
# Override with `MANIFEST_VERSION=N ./generate_manifest.sh` to force a specific
# value (useful for retries / partial reruns). Otherwise we list the public S3
# bucket via anonymous HTTP, find the highest existing `manifest_vN_*.omnimfst`,
# and bump by one. We use the public S3 bucket as the source of truth because
# (a) it's public so we don't need credentials, and (b) by convention all
# backends are kept on the same version.
#
# After generating + uploading, remember to update test references in
# `storage-core-integration-tests/tests/manifest.rs` to the new version.
detect_next_version() {
    local public_url="$S3_PUBLIC_URL"
    local current
    current=$(curl -sf "${public_url}/?list-type=2&prefix=${MANIFEST_SUBPATH}/manifest_v" 2>/dev/null \
        | sed -nE 's/.*manifest_v([0-9]+)_(minimal|full)\.omnimfst.*/\1/p' \
        | sort -n \
        | tail -1)
    # Default to 1 (so first run produces v2; matches existing convention) if nothing
    # is found — but in practice the bucket always has at least v2 today.
    echo "$((${current:-1} + 1))"
}

if [ -n "${MANIFEST_VERSION:-}" ]; then
    NEW_VERSION="$MANIFEST_VERSION"
    echo "Using forced manifest version: v${NEW_VERSION}"
else
    NEW_VERSION="$(detect_next_version)"
    echo "Auto-detected next manifest version: v${NEW_VERSION}"
fi

upload_manifest() {
    local file="$1"
    local dest_url="$2"
    local port="$3"

    local file_size
    file_size=$(wc -c < "$file")
    local file_base64
    file_base64=$(base64 -w 0 "$file")

    echo "  Uploading to ${dest_url} (${file_size} bytes)"

    printf '{"params":{"destinationResourceAddress":"%s","dataObjectSize":%d,"uploadPreference":"UPLOAD_PREFERENCE_BODY"}}\n{"chunk":{"chunk":"%s"}}\n' \
        "$dest_url" "$file_size" "$file_base64" | \
    grpcurl --plaintext -d @ \
        "localhost:${port}" \
        nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService.Write
}

# NOTE: the "_minimal" manifest variant (identity-only, no size/mtime) is no
# longer generated since identities became etag-only. Before that the identity URL
# embedded content-length + last-modified, so minimal and full were equivalent
# to consumers and minimal was just a JSON-size optimization. Now
# identity carries only etag + version-id, so minimal would force consumers to
# do a stat-per-object to get metadata — the wrong trade-off for a test
# fixture. The parsing code still accepts identity-only entries for backward
# compat with any pre-existing `_minimal.omnimfst` files in buckets; we just
# stop producing new ones.
for i in "${!LABELS[@]}"; do
    label="${LABELS[$i]}"
    url="${URLS[$i]}"
    port="${PORTS[$i]}"
    base_url="${url}/${MANIFEST_SUBPATH}"

    OUTPUT_FULL="manifest_${label}_v${NEW_VERSION}_full.omnimfst"

    echo "=== Processing: ${label} (${base_url}) ==="

    HEADER=$(cat <<EOF
{
  "format_version": {
    "version_string": "1.0"
  },
  "storage_api_version": "${STORAGE_API_VERSION}",
  "files": [
EOF
)

    echo "$HEADER" > "$OUTPUT_FULL"

    first=true
    for path in "${RELATIVE_PATHS[@]}"; do
        echo "  Processing: $path"

        resource_address="${base_url}/${path}"

        response=$(grpcurl --plaintext -d "{\"resource_address\": \"${resource_address}\"}" \
            "localhost:${port}" \
            nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService.Stat)

        resource_identity=$(echo "$response" | jq -r '.resourceInfo.resourceIdentity.encodedIdentity')
        size=$(echo "$response" | jq -r '.resourceInfo.metadata.dataObjectSize')
        modified_time=$(echo "$response" | jq -r '.resourceInfo.metadata.lastModifiedTimestamp')

        if [ -z "$resource_identity" ] || [ "$resource_identity" == "null" ]; then
            echo "  Warning: Failed to get resource identity for $path"
            continue
        fi

        if [ "$first" = false ]; then
            echo "," >> "$OUTPUT_FULL"
        fi
        first=false

        printf '    {\n      "relative_path": "%s",\n      "resource_info": {\n        "resource_identity": "%s",\n        "size": %s,\n        "modified_time": "%s"\n      }\n    }' \
            "$path" "$resource_identity" "$size" "$modified_time" >> "$OUTPUT_FULL"
    done

    FOOTER=$(cat <<EOF

  ]
}
EOF
)

    echo "$FOOTER" >> "$OUTPUT_FULL"
    echo "  Generated: $OUTPUT_FULL"

    # Upload manifests to the respective backend via the storage service. The
    # remote filename intentionally drops the per-backend label so all backends
    # expose the same logical filename (`manifest_vN_full.omnimfst`); the
    # difference between backends lives inside the manifest contents (each one
    # carries resource identities pointing at its own bucket/container).
    if [ "${WRITABLE[$i]}" = true ]; then
        upload_manifest "$OUTPUT_FULL" "${base_url}/manifest_v${NEW_VERSION}_full.omnimfst" "$port"
    else
        echo "  Skipping upload (${label} is read-only). Upload manually:"
        echo "    ${OUTPUT_FULL} -> ${base_url}/manifest_v${NEW_VERSION}_full.omnimfst"
    fi

    echo ""
done

echo "Done! All v${NEW_VERSION} manifests generated and uploaded (except read-only backends listed above)."
echo ""
echo "Next: update test references in storage-core-integration-tests/tests/manifest.rs"
echo "to use manifest_v${NEW_VERSION}_full.omnimfst."
