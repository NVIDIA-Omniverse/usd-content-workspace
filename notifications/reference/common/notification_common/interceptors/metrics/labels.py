# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Utility functions for creating metrics labels for gRPC methods.

These functions help parse gRPC method names and create standardized
OpenTelemetry-compliant labels for metrics.
"""

from typing import Dict, Final, Set, Tuple

from grpc import StatusCode
from opentelemetry.semconv._incubating.attributes.rpc_attributes import (
    RPC_METHOD,
    RPC_SERVICE,
    RPC_SYSTEM,
    RPC_GRPC_STATUS_CODE,
)

# Custom attribute for server errors
ATTR_RPC_GRPC_SERVER_ERROR: Final = "rpc.grpc.server_error"

# Server error status codes that indicate server-side failures
SERVER_ERROR_CODES: Set[StatusCode] = {
    StatusCode.INTERNAL,
    StatusCode.UNAVAILABLE,
    StatusCode.DATA_LOSS,
    StatusCode.UNIMPLEMENTED,
    StatusCode.DEADLINE_EXCEEDED,
    StatusCode.ABORTED,
    StatusCode.RESOURCE_EXHAUSTED,
}


def create_labels(method_name: str) -> Dict[str, str]:
    """
    Create standard labels for metrics following OpenTelemetry conventions.
    """
    service, method = _parse_method_name(method_name)
    return {
        RPC_SERVICE: service,
        RPC_METHOD: method,
        RPC_SYSTEM: "grpc",
    }


def create_status_labels(method_name: str, status_code: StatusCode) -> Dict[str, str]:
    """
    Create labels including status code information.
    """
    labels = create_labels(method_name)

    # Handle the case where status_code is None
    if status_code is None:
        status_code = StatusCode.UNKNOWN

    # Add status code as a string
    status_value = str(status_code.value[0])
    labels[RPC_GRPC_STATUS_CODE] = status_value

    # Add server error flag
    is_server_error = status_code in SERVER_ERROR_CODES
    labels[ATTR_RPC_GRPC_SERVER_ERROR] = str(is_server_error).lower()

    return labels


def _parse_method_name(method_name: str) -> Tuple[str, str]:
    """
    Parse a gRPC method name into service and method parts.
    """
    # Strip leading slash and split into parts
    clean_path = method_name.lstrip("/")
    parts = clean_path.split("/", 1)

    if len(parts) != 2:
        return ("unknown", clean_path)

    service_part, method = parts

    # Extract service name from package if present
    service = service_part.split(".")[-1] if "." in service_part else service_part

    return (service, method)
