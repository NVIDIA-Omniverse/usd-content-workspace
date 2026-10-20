# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import pytest
from grpc import StatusCode

from notification_common.interceptors.metrics.labels import (
    create_labels,
    create_status_labels,
    _parse_method_name,
    ATTR_RPC_GRPC_SERVER_ERROR,
)


class TestCreateLabels:
    @pytest.mark.parametrize(
        "method_name,expected_service,expected_method",
        [
            ("/service.Service/Method", "Service", "Method"),
            ("/package.subpackage.Service/Method", "Service", "Method"),
            ("Method", "unknown", "Method"),
            ("", "unknown", ""),
        ],
        ids=[
            "standard_method_name",
            "nested_service_name",
            "missing_service",
            "empty_string",
        ],
    )
    def test_create_labels(self, method_name, expected_service, expected_method):
        """Test that method names are parsed correctly in different formats."""
        labels = create_labels(method_name)
        assert labels["rpc.service"] == expected_service
        assert labels["rpc.method"] == expected_method
        assert labels["rpc.system"] == "grpc"


class TestCreateStatusLabels:
    def test_status_labels_with_ok_status(self):
        """Test that OK status is handled correctly."""
        labels = create_status_labels("/service.Service/Method", StatusCode.OK)
        assert labels["rpc.service"] == "Service"
        assert labels["rpc.method"] == "Method"
        assert labels["rpc.system"] == "grpc"
        assert "rpc.grpc.status_code" in labels
        assert labels[ATTR_RPC_GRPC_SERVER_ERROR] == "false"

    @pytest.mark.parametrize(
        "status_code,is_server_error",
        [
            (StatusCode.INVALID_ARGUMENT, False),
            (StatusCode.PERMISSION_DENIED, False),
            (StatusCode.UNAUTHENTICATED, False),
            (StatusCode.INTERNAL, True),
            (StatusCode.UNAVAILABLE, True),
            (StatusCode.RESOURCE_EXHAUSTED, True),
            (StatusCode.DEADLINE_EXCEEDED, True),
        ],
        ids=[
            "invalid_argument",
            "permission_denied",
            "unauthenticated",
            "internal",
            "unavailable",
            "resource_exhausted",
            "deadline_exceeded",
        ],
    )
    def test_status_labels_with_various_error_codes(self, status_code, is_server_error):
        """Test that different error codes are marked correctly."""
        labels = create_status_labels("/service.Service/Method", status_code)
        assert "rpc.grpc.status_code" in labels
        assert labels[ATTR_RPC_GRPC_SERVER_ERROR] == str(is_server_error).lower()


class TestParseMethodName:
    @pytest.mark.parametrize(
        "method_name,expected_service,expected_method",
        [
            ("/service.Service/Method", "Service", "Method"),
            ("/a.b.c.Service/Method", "Service", "Method"),
            ("service.Service/Method", "Service", "Method"),
            ("/Service/Method", "Service", "Method"),
            ("InvalidMethod", "unknown", "InvalidMethod"),
            ("", "unknown", ""),
        ],
        ids=[
            "standard_method_name",
            "multiple_dots_in_service",
            "no_leading_slash",
            "no_package",
            "invalid_format",
            "empty_string",
        ],
    )
    def test_parse_method_name(self, method_name, expected_service, expected_method):
        """Test parsing of method names in various formats."""
        service, method = _parse_method_name(method_name)
        assert service == expected_service
        assert method == expected_method
