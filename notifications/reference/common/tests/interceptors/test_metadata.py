# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import pytest

from notification_common.interceptors.metadata import (
    extract_metadata_value,
    metadata_string_value,
)


class TestExtractMetadataValue:

    @pytest.fixture
    def mock_metadata(self):
        return [
            ("content-type", "application/grpc"),
            ("user-agent", "grpc-python/1.50.0"),
        ]

    def test_returns_metadata_value_for_key(self, mock_metadata):
        assert (
            extract_metadata_value(mock_metadata, "user-agent") == "grpc-python/1.50.0"
        )

    def test_returns_none_if_key_not_found(self, mock_metadata):
        assert extract_metadata_value(mock_metadata, "non-existent-key") is None

    def test_returns_none_if_empty_metadata(self):
        assert extract_metadata_value([], "any-key") is None


class TestMetadataStringValue:

    def test_input_is_string(self):
        input_str = "already a string"
        assert metadata_string_value(input_str) == input_str

    def test_returns_string_from_bytes(self):
        input_bytes = b"convert this bytes"
        expected_str = "convert this bytes"
        assert metadata_string_value(input_bytes) == expected_str

    def test_returns_string_from_bytearray(self):
        input_bytearray = bytearray(b"convert this bytearray")
        expected_str = "convert this bytearray"
        assert metadata_string_value(input_bytearray) == expected_str

    def test_returns_string_from_memoryview(self):
        input_memoryview = memoryview(b"convert this memoryview")
        expected_str = "convert this memoryview"
        assert metadata_string_value(input_memoryview) == expected_str

    def test_raises_decoding_error_if_invalid_utf8(self):
        input_bytes = b"\xff\xfeinvalid"
        with pytest.raises(UnicodeDecodeError):
            metadata_string_value(input_bytes)
