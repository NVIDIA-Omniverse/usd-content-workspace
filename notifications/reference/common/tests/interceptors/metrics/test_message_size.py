# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import pytest
from unittest.mock import MagicMock

from notification_common.interceptors.metrics.message_size import get_message_size


class TestGetMessageSize:
    def test_none_message_returns_zero(self):
        """Test that None messages return size 0."""
        size = get_message_size(None)
        assert size == 0

    def test_message_with_byte_size_method(self):
        """Test that messages with ByteSize() methods return correct size."""
        mock_message = MagicMock()
        mock_message.ByteSize.return_value = 123

        size = get_message_size(mock_message)
        assert size == 123
        mock_message.ByteSize.assert_called_once()

    def test_message_with_sizeof_method(self):
        """Test that messages with __sizeof__() method return correct size."""
        mock_message = MagicMock()
        # Make ByteSize raise an exception to test the fallback
        mock_message.ByteSize.side_effect = Exception("No ByteSize")
        mock_message.__sizeof__.return_value = 456

        size = get_message_size(mock_message)
        assert size == 456
        mock_message.__sizeof__.assert_called_once()

    @pytest.mark.parametrize(
        "default_size",
        [0, 10, 999],
        ids=["default_zero", "default_small", "default_large"],
    )
    def test_message_with_no_size_methods(self, default_size):
        """Test that messages with no size methods return the default size."""
        mock_message = MagicMock()
        # Make both methods raise exceptions
        mock_message.ByteSize.side_effect = Exception("No ByteSize")
        mock_message.__sizeof__.side_effect = Exception("No __sizeof__")

        size = get_message_size(mock_message, default_size=default_size)
        assert size == default_size

    @pytest.mark.parametrize(
        "primitive_value",
        ["test string", 12345, [1, 2, 3, 4, 5], {"key": "value"}, (1, 2, 3)],
        ids=["string", "integer", "list", "dict", "tuple"],
    )
    def test_primitive_types(self, primitive_value):
        """Test handling of primitive types without special size methods."""
        # For strings, ints, and other primitives, __sizeof__ should be used
        assert get_message_size(primitive_value) > 0

    def test_protobuf_message_simulation(self):
        """Test with an object that simulates a protobuf message."""

        class MockProtobuf:
            def ByteSize(self):
                return 789

        mock_proto = MockProtobuf()
        size = get_message_size(mock_proto)
        assert size == 789
