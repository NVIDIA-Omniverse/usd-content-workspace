# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import pytest

from notification_common.auth.auth_context import auth_token_var


class TestAuthTokenVar:
    def test_get_unset_raises_lookup_error(self):
        with pytest.raises(LookupError):
            auth_token_var.get()

    def test_get_retrieves_set_value(self):
        test_token = "my-test-token"
        reset_token = auth_token_var.set(test_token)

        assert auth_token_var.get() == test_token

        auth_token_var.reset(reset_token)
