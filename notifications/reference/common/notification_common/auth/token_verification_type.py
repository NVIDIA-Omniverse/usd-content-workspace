# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from enum import Enum


class TokenVerificationType(Enum):
    JWT = "jwt"
    OPAQUE = "opaque"
