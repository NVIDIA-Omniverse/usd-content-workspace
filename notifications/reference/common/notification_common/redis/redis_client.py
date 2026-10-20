# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import redis.asyncio as redis

from notification_common.config.common_settings import common_settings

"""
Need to be able to handle redis uris that look like:

redis://localhost:6379

or

localhost:6379

or with credentials:

name:password@host:port

or come from Azure Cache for Redis connection strings and look like:

host:port,password=password,ssl=True,abortConnect=False"
"""


async def close_redis_client() -> None:
    if redis_client:
        await redis_client.aclose()

    if decoding_redis_client:
        await decoding_redis_client.aclose()


def _get_redis_client(uri: str = None, decode_responses=False) -> redis.Redis | None:
    redis_uri = uri if uri else common_settings.redis_uri

    if not redis_uri:
        return None

    # don't need the scheme in the uri
    redis_uri = redis_uri.split("://")[1] if "://" in redis_uri else redis_uri

    host, port, password, is_ssl = "", "", None, False

    for part in redis_uri.split(","):
        if "=" in part:
            part_value = part.split("=", 1)[1]

            if "password" in part:
                password = part_value
            elif "ssl" in part:
                is_ssl = part_value.lower() == "true"
        else:
            # Handle formats like name:password@host:port or host:port
            if "@" in part:
                auth_part, host_part = part.split("@", 1)
                # Extract password from name:password
                if ":" in auth_part:
                    _, password = auth_part.split(":", 1)
                # Extract host and port from host:port
                host, port = host_part.split(":")
            else:
                host, port = part.split(":")

    return redis.Redis(
        host=host,
        port=int(port),
        password=password,
        ssl=is_ssl,
        socket_connect_timeout=10,
        decode_responses=decode_responses,
    )


redis_client = _get_redis_client()

decoding_redis_client = _get_redis_client(decode_responses=True)
