# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import pytest
from notification_common.rabbitmq.publisher import FailedPublishingResult

# noinspection PyProtectedMember
from app.grpc.grpc_publishing_results import (
    grpc_publishing_result,
    grpc_publishing_results,
)

from nvidia.omniverse.notifications.publisher.v1beta.event_publisher_pb2 import (
    PublishingResult,
)


class TestGrpcPublishingResult:
    @pytest.fixture
    def result(self, mocked_omni_event, mocked_event) -> PublishingResult:
        return grpc_publishing_result(
            FailedPublishingResult(event=mocked_omni_event, reason="test-reason"),
            mocked_event,
        )

    def test_takes_publishing_result_and_returns_grpc_publishing_result(
        self, mocked_event, result
    ) -> None:
        assert isinstance(result, PublishingResult)

    def test_result_include_original_event(self, result, mocked_event) -> None:
        assert result.event == mocked_event

    def test_result_includes_success_bool(self, result, mocked_event) -> None:
        assert result.success is False

    def test_includes_failure_reason(self, result: PublishingResult) -> None:
        assert result.failure_reason == "test-reason"


class TestGrpcPublishingResults:
    @pytest.fixture
    def results(self, mocked_omni_event, mocked_event) -> list[PublishingResult]:
        arg = [
            FailedPublishingResult(event=mocked_omni_event, reason="test-reason")
            for _ in range(3)
        ]
        return grpc_publishing_results(arg, [mocked_event, mocked_event, mocked_event])

    def test_returns_list_of_grpc_publishing_results(self, results) -> None:
        assert len(results) == 3

        for result in results:
            assert isinstance(result, PublishingResult)
