#!/bin/bash
# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

METRICS_URL="http://localhost:8002/metrics"
SCRAPE_OUTPUT="/tmp/scrape.txt"

# Scrape and save metrics output
curl -s "$METRICS_URL" > "$SCRAPE_OUTPUT"

echo "=== storage_<operation>_requests_counter (non-SDK) ==="

# Extract operations for non-SDK requests_counter (exclude lines with _sdk_)
operations=$(grep -E '^storage_.*_requests_counter\{' "$SCRAPE_OUTPUT" | \
             grep -v '_sdk_requests_counter' | \
             sed -E 's/^storage_(.*)_requests_counter\{.*/\1/' | \
             sort -u)

grand_total=0

for op in $operations; do
    sum=$(grep -E "^storage_${op}_requests_counter\{.*\} " "$SCRAPE_OUTPUT" | \
          awk '{sum += $2} END {print sum+0}')
    echo "  ${op}: ${sum}"
    grand_total=$(echo "$grand_total + $sum" | bc)
done

echo "  TOTAL: ${grand_total}"
echo ""

echo "=== storage_<operation>_sdk_requests_counter ==="

# Extract operations for SDK requests
operations_sdk=$(grep -E '^storage_.*_sdk_requests_counter\{' "$SCRAPE_OUTPUT" | \
                 sed -E 's/^storage_(.*)_sdk_requests_counter\{.*/\1/' | \
                 sort -u)

grand_total_sdk=0

for op in $operations_sdk; do
    sum=$(grep -E "^storage_${op}_sdk_requests_counter\{.*\} " "$SCRAPE_OUTPUT" | \
          awk '{sum += $2} END {print sum+0}')
    echo "  ${op}: ${sum}"
    grand_total_sdk=$(echo "$grand_total_sdk + $sum" | bc)
done

echo "  TOTAL: ${grand_total_sdk}"
