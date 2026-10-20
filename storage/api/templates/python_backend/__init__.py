# SPDX-FileCopyrightText: Copyright (c) 2025-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.
#
# This file is a template. NVIDIA distributes it under Apache-2.0 so that you may
# copy it into your own project. When you copy it, keep the notice above and add
# your own SPDX-FileCopyrightText line alongside it -- Apache-2.0 requires the
# original notices to travel with distributed derivatives. If you release your
# derivative under a different license, change the SPDX-License-Identifier and the
# license notice text together so the two agree. See the "Licensing" section of
# README.md for details.
#
# Package initialization and CLI registration for MyStorage backend.

"""
MyStorage backend package for Omniverse Storage API.

This package provides a storage backend implementation for [YOUR STORAGE SYSTEM].
"""

import click
from local_filesystem_service.backends.cli_registry import register_backend_cli

# Import the provider to trigger backend registration
from . import my_storage_provider  # noqa: F401

# =============================================================================
# CLI Registration
# =============================================================================
# This decorator registers CLI options that appear when users run:
#   local-filesystem-service mystorage --help


@register_backend_cli("mystorage")  # <-- Must match the name in @register_backend
@click.option(
    "--option1",
    default="default_value",
    envvar="MYSTORAGE_OPTION1",
    help="Description of option1",
    show_default=True,
)
@click.option(
    "--option2/--no-option2",
    default=False,
    envvar="MYSTORAGE_OPTION2",
    help="A boolean flag option",
    show_default=True,
)
@click.option(
    "--base-uri",
    default="mystorage://default",
    envvar="MYSTORAGE_BASE_URI",
    help="Base URI for resource addresses",
    show_default=True,
)
def mystorage_cli(option1: str, option2: bool, base_uri: str):
    """MyStorage backend configuration.

    This docstring appears in --help output.

    Example:
        local-filesystem-service mystorage --option1 myvalue --base-uri mystorage://mybucket
    """
    # Return a dict that will be passed to the backend factory as extra_config
    return {
        "option1": option1,
        "option2": option2,
        "base_uri": base_uri,
    }


# =============================================================================
# Example: S3-style configuration
# =============================================================================
# Uncomment and modify for S3-like storage:
#
# @register_backend_cli("s3")
# @click.option("--bucket", required=True, help="S3 bucket name")
# @click.option("--region", default="us-east-1", help="AWS region")
# @click.option("--endpoint-url", default=None, help="Custom endpoint URL (for MinIO, etc.)")
# @click.option("--base-uri", default="s3://bucket", help="Base URI for addresses")
# def s3_cli(bucket: str, region: str, endpoint_url: str, base_uri: str):
#     """AWS S3 storage backend."""
#     return {
#         "bucket": bucket,
#         "region": region,
#         "endpoint_url": endpoint_url,
#         "base_uri": base_uri,
#     }


# =============================================================================
# Example: Azure Blob Storage configuration
# =============================================================================
# Uncomment and modify for Azure:
#
# @register_backend_cli("azure")
# @click.option("--account-name", required=True, envvar="AZURE_STORAGE_ACCOUNT")
# @click.option("--container", required=True, help="Azure container name")
# @click.option("--connection-string", envvar="AZURE_STORAGE_CONNECTION_STRING")
# @click.option("--base-uri", default="azure://container", help="Base URI")
# def azure_cli(account_name: str, container: str, connection_string: str, base_uri: str):
#     """Azure Blob Storage backend."""
#     return {
#         "account_name": account_name,
#         "container": container,
#         "connection_string": connection_string,
#         "base_uri": base_uri,
#     }
