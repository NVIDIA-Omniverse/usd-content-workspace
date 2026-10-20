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

# Test script for create endpoint with multiple image_to_image specifications
# This demonstrates how the endpoint can handle multiple specifications in a single request

# Configuration
HTTP_ENDPOINT="http://localhost:3000"
BEARER_TOKEN="your-auth-token-here"

echo "=== MULTIPLE SPECIFICATIONS TEST ==="
echo "Testing POST /thumbnails/create with multiple image_to_image specifications"
echo ""

# Test 1: Multiple image_to_image specifications
echo "Test 1: Multiple image_to_image specifications"
JSON_PAYLOAD_1=$(cat <<EOF
{
  "resource_identity": "https://example.com/test-image.jpg",
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
    },
    {
      "image_to_image": {
        "processor_identity": "PROCESSOR_IDENTITY_IMAGE_TO_IMAGE",
        "encoding_format": "IMAGE_ENCODING_FORMAT_JPEG",
        "width": 512,
        "height": 512,
        "transform_type": "TRANSFORM_TYPE_FIT",
        "crop_direction": "CROP_DIRECTION_CENTER"
      }
    },
    {
      "image_to_image": {
        "processor_identity": "PROCESSOR_IDENTITY_IMAGE_TO_IMAGE",
        "encoding_format": "IMAGE_ENCODING_FORMAT_WEBP",
        "width": 128,
        "height": 128,
        "transform_type": "TRANSFORM_TYPE_SQUARE",
        "crop_direction": "CROP_DIRECTION_BOTTOM"
      }
    }
  ]
}
EOF
)

echo "Request payload:"
echo "$JSON_PAYLOAD_1" | jq .
echo ""

echo "Expected response:"
echo '{
  "thumbnails": [
    {
      "resource_identity": "https://example.com/test-image.jpg",
      "task_id": "task-uuid-1",
      "status": "PENDING",
      "specification": { ... },
      "create_time": "2024-01-01T00:00:00Z",
      "error": null
    },
    {
      "resource_identity": "https://example.com/test-image.jpg",
      "task_id": "task-uuid-2",
      "status": "PENDING",
      "specification": { ... },
      "create_time": "2024-01-01T00:00:00Z",
      "error": null
    },
    {
      "resource_identity": "https://example.com/test-image.jpg",
      "task_id": "task-uuid-3",
      "status": "PENDING",
      "specification": { ... },
      "create_time": "2024-01-01T00:00:00Z",
      "error": null
    }
  ]
}'
echo ""

echo "To run the actual test:"
echo "curl -X POST \"$HTTP_ENDPOINT/thumbnails/create\" \\"
echo "  -H \"Content-Type: application/json\" \\"
echo "  -H \"Authorization: Bearer $BEARER_TOKEN\" \\"
echo "  -d '$JSON_PAYLOAD_1'"
echo ""

# Test 2: Mixed specification types
echo "Test 2: Mixed specification types (image_to_image + usd_to_image)"
JSON_PAYLOAD_2=$(cat <<EOF
{
  "resource_identity": "https://example.com/test-file.usd",
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
    },
    {
      "usd_to_image": {
        "processor_identity": "PROCESSOR_IDENTITY_USD_TO_IMAGE",
        "encoding_format": "IMAGE_ENCODING_FORMAT_PNG",
        "width": 512,
        "height": 512,
        "transform_type": "TRANSFORM_TYPE_SQUARE",
        "crop_direction": "CROP_DIRECTION_CENTER",
        "stage_name": "defaultStage",
        "render_product": "defaultRenderProduct"
      }
    }
  ]
}
EOF
)

echo "Request payload:"
echo "$JSON_PAYLOAD_2" | jq .
echo ""

echo "To run the mixed specifications test:"
echo "curl -X POST \"$HTTP_ENDPOINT/thumbnails/create\" \\"
echo "  -H \"Content-Type: application/json\" \\"
echo "  -H \"Authorization: Bearer $BEARER_TOKEN\" \\"
echo "  -d '$JSON_PAYLOAD_2'"
echo ""

# Test 3: Empty specifications array
echo "Test 3: Empty specifications array (edge case)"
JSON_PAYLOAD_3=$(cat <<EOF
{
  "resource_identity": "https://example.com/test-image.jpg",
  "specification": []
}
EOF
)

echo "Request payload:"
echo "$JSON_PAYLOAD_3" | jq .
echo ""

echo "Expected response: Empty thumbnails array"
echo '{
  "thumbnails": []
}'
echo ""

echo "To run the empty specifications test:"
echo "curl -X POST \"$HTTP_ENDPOINT/thumbnails/create\" \\"
echo "  -H \"Content-Type: application/json\" \\"
echo "  -H \"Authorization: Bearer $BEARER_TOKEN\" \\"
echo "  -d '$JSON_PAYLOAD_3'"
echo ""

echo "=== CONFIGURATION NOTES ==="
echo "The service now supports array-based configuration via:"
echo "- THUMBNAIL_DEFAULT_SPECIFICATIONS environment variable (JSON array)"
echo "- Helm ConfigMap with THUMBNAIL_DEFAULT_SPECIFICATIONS"
echo "- Docker ENV with THUMBNAIL_DEFAULT_SPECIFICATIONS"
echo ""
echo "Legacy individual settings are still supported for backward compatibility."
echo "See config.example for configuration details."
