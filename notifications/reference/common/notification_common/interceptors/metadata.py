# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from typing import Union, Sequence, Tuple
import logging

logger = logging.getLogger(__name__)

GrpcMetadataKey = str
GrpcMetadataValue = Union[str, bytes, bytearray, memoryview]


def extract_metadata_value(
    metadata: Sequence[Tuple[GrpcMetadataKey, GrpcMetadataValue]], key: str
) -> GrpcMetadataValue | None:
    for k, v in metadata:
        if k == key:
            return v


def metadata_string_value(
    metadata_value: GrpcMetadataValue,
) -> str:

    if (
        isinstance(metadata_value, bytes)
        or isinstance(metadata_value, bytearray)
        or isinstance(metadata_value, memoryview)
    ):
        if isinstance(metadata_value, memoryview):
            metadata_value = bytes(metadata_value)

        try:
            return metadata_value.decode("utf-8")
        except UnicodeDecodeError:
            logger.exception(f"Failed to decode value {metadata_value} as UTF-8.")
            raise

    else:
        return metadata_value
