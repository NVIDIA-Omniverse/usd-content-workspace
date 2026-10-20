# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import pytest


@pytest.fixture
def jwks_uri():
    return "test-jwks-uri"


@pytest.fixture
def test_user_id() -> str:
    return "user-from-decoder"
