# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from enum import Enum

from notification_common.errors.errors import UnprocessableError


class FilterType(Enum):
    """
    FilterType specifies the matching strategy for the resource filter.

    - unspecified: The default, unspecified filter type. This will
      default to eq behavior.
    - eq: Performs an exact match on the resource_id.
    - starts_with_lazy: Performs a "starts with" match (lazy).
      For example, a filter for "folder_1" would match
      "folder_1/file.txt" but not "folder_10/file.txt".
    - starts_with_greedy: Performs a "starts with" match (greedy).
      For example, a filter for "folder_1" would match
      "folder_1/sub/file.txt".
    """

    UNSPECIFIED = "unspecified"
    EQ = "eq"
    STARTS_WITH_LAZY = "starts_with_lazy"
    STARTS_WITH_GREEDY = "starts_with_greedy"

    @classmethod
    def _missing_(cls, value):
        # Handle integer values from gRPC
        if isinstance(value, int):
            mapping = {
                0: cls.UNSPECIFIED,
                1: cls.EQ,
                2: cls.STARTS_WITH_LAZY,
                3: cls.STARTS_WITH_GREEDY,
            }
            if value in mapping:
                return mapping[value]

        # Handle old uppercase string values for backward compatibility
        if isinstance(value, str):
            uppercase_mapping = {
                "UNSPECIFIED": cls.UNSPECIFIED,
                "EQ": cls.EQ,
                "STARTS_WITH_LAZY": cls.STARTS_WITH_LAZY,
                "STARTS_WITH_GREEDY": cls.STARTS_WITH_GREEDY,
            }
            if value in uppercase_mapping:
                return uppercase_mapping[value]

        raise UnprocessableError(f"{value} is not a valid FilterType")
