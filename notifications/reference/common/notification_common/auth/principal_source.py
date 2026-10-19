# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from enum import Enum


class PrincipalSource(Enum):
    DISABLED = "disabled"
    JWT_UNVERIFIED = "jwtUnverified"
    JWT = "jwt"
    OPAQUE = "opaque"
    USER_INFO_SERVICE = "userInfoService"
