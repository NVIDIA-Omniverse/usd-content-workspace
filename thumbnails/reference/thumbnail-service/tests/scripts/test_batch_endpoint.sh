#!/bin/bash
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

# Test script for the batch endpoint
# This shows exactly what gets sent to the /thumbnails/utils/create-for-files-in-folder-batch endpoint

FOLDER_URI="https://checkpointblobs.blob.core.windows.net/forthumbnails/demo/1000%2B%20images/"
# Load the bearer token from the environment; never hardcode credentials in the repo.
# Export BEARER_TOKEN (e.g. from a CI secret) before running this script.
BEARER_TOKEN="${BEARER_TOKEN:?BEARER_TOKEN environment variable must be set (export it from a secret; do not hardcode)}"

# Create the JSON payload
JSON_PAYLOAD=$(cat <<EOF
{
  "folder_uri": "$FOLDER_URI",
  "specification": [
    {
      "image_to_image": {
        "processor_identity": "PROCESSOR_IDENTITY_IMAGE_TO_IMAGE",
        "encoding_format": "IMAGE_ENCODING_FORMAT_PNG",
        "width": 256,
        "height": 256,
        "transform_type": "TRANSFORM_TYPE_SQUARE",
        "crop_direction": "CROP_DIRECTION_CENTER"
      }
    }
  ]
}
EOF
)

echo "=== BATCH ENDPOINT TEST ==="
echo "Endpoint: POST /thumbnails/utils/create-for-files-in-folder-batch"
echo "Folder URI: $FOLDER_URI"
echo ""
echo "=== REQUEST HEADERS ==="
echo "Content-Type: application/json"
echo "Authorization: Bearer [TOKEN_HIDDEN_FOR_SECURITY]"
echo ""
echo "=== REQUEST BODY ==="
echo "$JSON_PAYLOAD" | jq .
echo ""
echo "=== WHAT THE ENDPOINT WILL DO ==="
echo "1. Extract authorization header from request"
echo "2. Call list_files_in_folder() with folder_uri: $FOLDER_URI"
echo "3. For each file found, include both resource_address and resource_identity from source metadata"
echo "4. Create batch requests with resource_identity (not resource_address)"
echo "5. Submit batch to task queue"
echo "6. Return task_ids for tracking"
echo ""
echo "=== TO RUN THE ACTUAL REQUEST ==="
echo "curl -X POST \"http://localhost:3000/thumbnails/utils/create-for-files-in-folder-batch\" \\"
echo "  -H \"Content-Type: application/json\" \\"
echo "  -H \"Authorization: Bearer \$BEARER_TOKEN\" \\"
echo "  -d '$JSON_PAYLOAD'"
echo ""
echo "=== EXPECTED RESPONSE FORMAT ==="
echo '{
  "task_ids": ["task-uuid-1", "task-uuid-2", "..."],
  "count": 1234,
  "status": "queued",
  "message": "Batch of 1234 tasks queued for creation"
}'
