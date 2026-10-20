# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import pytest
from pydantic import ValidationError

from app.rest.models.channel_pool_metrics import (
    _INT32_MAX,
    ChannelPoolMetricsResponse,
)

# The upper bound this Task retired from both count fields. Referenced only so
# the cap-scale cases below read as "past the bound that used to be here".
RETIRED_COUNT_UPPER_BOUND = 10_000

# The operator-set memory cap used as the worked example in the Task: a
# max_non_durable_streams_per_pod an operator can legitimately configure.
OPERATOR_CAP_EXAMPLE = 20_000


class TestChannelPoolMetricsResponse:
    def test_creates_response_with_all_fields(self):
        response = ChannelPoolMetricsResponse(
            utilization_percent=75,
            channels_in_use=75,
            channels_total=100,
        )

        assert response.utilization_percent == 75
        assert response.channels_in_use == 75
        assert response.channels_total == 100

    def test_serializes_to_dict(self):
        response = ChannelPoolMetricsResponse(
            utilization_percent=50,
            channels_in_use=50,
            channels_total=100,
        )

        result = response.model_dump()

        assert result == {
            "utilization_percent": 50,
            "channels_in_use": 50,
            "channels_total": 100,
        }


class TestChannelPoolMetricsResponseFieldBounds:
    """
    Both count fields now carry a NON-BINDING int32 sentinel maximum; ``ge=0``
    and ``utilization_percent``'s ``le=100`` survive unchanged.

    Current state: ``channels_in_use`` and ``channels_total`` each declare
    ``le=_INT32_MAX`` (``2_147_483_647``) plus ``json_schema_extra={"format":
    "int32"}``, added solely to satisfy the OWASP ``integer-limit`` API
    governance rule (every integer schema must declare both a minimum and a
    maximum). ``ge=0`` is unchanged on both, and ``utilization_percent``'s
    ``le=100`` is untouched.

    The sentinel is non-binding: the real ceiling on the in-use count is the
    operator-settable memory cap (``max_non_durable_streams_per_pod``, in the
    low thousands), not the ``pool_size x client_limit`` packing product the old
    ``le=10000`` constant was sized against. Because no legitimate pod reports a
    value anywhere near int32-max, the sentinel never binds, and the old
    ``le=10000``-made-the-handler-non-200 hazard does not apply: that finite
    bound was removed (b.tj2 Epic 3) precisely because a real deployment could
    exceed it, take the metrics feed non-200, and drop the pod out of the
    autoscaler's fleet view when it is most loaded.
    """

    @pytest.mark.parametrize(
        "count",
        [
            RETIRED_COUNT_UPPER_BOUND + 1,
            OPERATOR_CAP_EXAMPLE,
            100_000,
        ],
    )
    def test_accepts_cap_scale_counts(self, count):
        response = ChannelPoolMetricsResponse(
            utilization_percent=100,
            channels_in_use=count,
            channels_total=count,
        )

        assert (response.channels_in_use, response.channels_total) == (count, count)

    @pytest.mark.parametrize("field", ["channels_in_use", "channels_total"])
    def test_rejects_negative_count(self, field):
        """
        ``ge=0`` survives, so the route's negative clamp stays load-bearing.

        The handler clamps a negative pool reading to zero before constructing
        this model; that clamp is only meaningful while the model would in fact
        reject a negative count.
        """
        values = {
            "utilization_percent": 0,
            "channels_in_use": 0,
            "channels_total": 0,
            field: -1,
        }

        with pytest.raises(ValidationError, match=field):
            ChannelPoolMetricsResponse(**values)

    def test_rejects_utilization_percent_above_100(self):
        """``le=100`` on the percentage is untouched by this Task."""
        with pytest.raises(ValidationError, match="utilization_percent"):
            ChannelPoolMetricsResponse(
                utilization_percent=101,
                channels_in_use=0,
                channels_total=0,
            )

    @pytest.mark.parametrize("field", ["channels_in_use", "channels_total"])
    def test_count_fields_declare_int32_sentinel_bounds_in_schema(self, field):
        """
        Both count fields must publish ``minimum: 0`` and ``maximum: _INT32_MAX``.

        The sentinel maximum is non-binding at runtime, so no value-level test
        exercises it; this pins the generated JSON schema instead. Removing the
        sentinel would silently re-break the OWASP ``integer-limit`` governance
        lint (integers must declare both a minimum and a maximum) -- this guard
        catches that here rather than in a CI governance scan.
        """
        schema = ChannelPoolMetricsResponse.model_json_schema()
        properties = schema["properties"][field]

        assert properties["minimum"] == 0
        assert properties["maximum"] == _INT32_MAX
