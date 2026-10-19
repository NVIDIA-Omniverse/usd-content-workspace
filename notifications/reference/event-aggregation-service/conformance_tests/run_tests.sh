#!/usr/bin/env sh
# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
#
# Run the Event Aggregation Service conformance tests.

set -e

SKIP_OPTIONAL="${SKIP_OPTIONAL_TESTS:-false}"

if [ "$SKIP_OPTIONAL" = "true" ]; then
    exec run-conformance-tests -m "not optional"
else
    exec run-conformance-tests
fi
