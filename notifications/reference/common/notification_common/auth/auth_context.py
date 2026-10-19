# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from contextvars import ContextVar

_AUTH_TOKEN_KEY = "auth_token"

auth_token_var: ContextVar[str] = ContextVar(_AUTH_TOKEN_KEY)
