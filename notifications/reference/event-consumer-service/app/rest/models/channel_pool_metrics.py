# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from pydantic import BaseModel, Field

# Sentinel int32 maximum. It exists only to satisfy the OWASP integer-limit API
# governance rule, which requires every integer schema to declare both a minimum
# and a maximum. It is NOT a real ceiling: a pod's per-request stream counts are
# bounded by the memory-safe cap (low thousands), so no legitimate value can
# approach int32-max and trip the response-model ValidationError. The old finite
# le=10000 was removed (b.tj2 Epic 3) precisely because a real deployment could
# exceed it, take the metrics feed non-200, and drop the pod from the autoscaler's
# fleet view when most loaded; the sentinel keeps the schema valid without that
# risk. `format: int32` is declared to match, clearing the OWASP integer-format
# warning for the same fields.
_INT32_MAX = 2_147_483_647
_INT32_FORMAT = {"format": "int32"}


class ChannelPoolMetricsResponse(BaseModel):
    """
    Response model for the channel pool metrics endpoint.

    Values are counted in non-durable stream slots, not channels.
    """

    utilization_percent: int = Field(
        ge=0,
        le=100,
        json_schema_extra=_INT32_FORMAT,
        description=(
            "Percentage of effective non-durable stream capacity in use (0-100)."
        ),
    )
    # channels_in_use and channels_total carry the non-binding _INT32_MAX sentinel
    # as their upper bound (see the module comment): they are deliberately left
    # effectively unclamped so the autoscaler sees the true load -- including
    # channels_in_use exceeding channels_total in the overflow band -- and the
    # sentinel is far above any value a legitimate pod can report.
    channels_in_use: int = Field(
        ge=0,
        le=_INT32_MAX,
        json_schema_extra=_INT32_FORMAT,
        description=(
            "Active streams on the non-durable pool, in stream slots. This may "
            "exceed channels_total while the pod is serving in the overflow "
            "band; it is deliberately left effectively unclamped (the maximum is "
            "a non-binding int32 sentinel) so the autoscaler sees the true load "
            "and the resulting negative available capacity."
        ),
    )
    channels_total: int = Field(
        ge=0,
        le=_INT32_MAX,
        json_schema_extra=_INT32_FORMAT,
        description=(
            "Effective non-durable stream capacity: that pool's stream "
            "capacity less the readiness reserve, in stream slots. The "
            "autoscaler uses this to determine usable available capacity."
        ),
    )

    model_config = {
        "json_schema_extra": {
            "examples": [
                {
                    "utilization_percent": 75,
                    "channels_in_use": 75,
                    "channels_total": 100,
                }
            ]
        }
    }
