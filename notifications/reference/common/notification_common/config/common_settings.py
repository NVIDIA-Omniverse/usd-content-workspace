# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import ipaddress
import json
import re
from typing import Annotated, Any, Iterable

from pydantic import Field, field_validator, model_validator
from pydantic_settings import NoDecode

from notification_common.config.notification_base_settings import (
    NotificationBaseSettings,
)
from notification_common.errors.errors import ServerError

# The form browsers send in the Origin header, which the CORS middleware
# compares as an exact string: http or https, a host, and a port only when it
# is not the scheme's default. See _is_serialized_origin for the host rules.
_ORIGIN_SHAPE = re.compile(
    r"(https?)://(\[[0-9a-f:.]+\]|[^:/?#\[\]@\s]+)(?::([1-9][0-9]{0,4}))?"
)
_DEFAULT_PORTS = {"http": 80, "https": 443}
_HOST_LABEL = re.compile(r"[a-z0-9_-]+")
# A last label like this makes a browser parse the whole host as IPv4.
_NUMERIC_LABEL = re.compile(r"[0-9]+|0x[0-9a-f]*")


def _serialize_ipv6(address: ipaddress.IPv6Address) -> str:
    """
    Serialize an IPv6 address the way the WHATWG URL standard does.

    ``address.compressed`` is not used because Python 3.13 and later print an
    IPv4-mapped address in dotted form, which browsers never send.
    """
    value = int(address)
    pieces = [format(value >> (16 * (7 - n)) & 0xFFFF, "x") for n in range(8)]

    # Find the first longest run of two or more zero pieces; it becomes "::".
    run_start, run_length = None, 1
    index = 0
    while index < len(pieces):
        length = 0
        while index + length < len(pieces) and pieces[index + length] == "0":
            length += 1
        if length > run_length:
            run_start, run_length = index, length
        index += max(length, 1)

    if run_start is None:
        return ":".join(pieces)
    run_end = run_start + run_length
    return ":".join(pieces[:run_start]) + "::" + ":".join(pieces[run_end:])


def _is_serialized_origin(origin: str) -> bool:
    """
    Return whether a lower-cased origin is in the exact form a browser sends.

    The host must be a bracketed IPv6 address or a dotted IPv4 address in
    the canonical form browsers serialize, or dot-separated labels of ASCII
    letters, digits, hyphens, and underscores (so IDNs must be given in
    punycode) whose last label is not numeric, since a browser would parse
    that host as IPv4. User info, wildcards, trailing dots, paths, queries,
    fragments, out-of-range ports, and the scheme's default port are all
    refused, because none of them can ever equal a browser's Origin header.
    """
    match = _ORIGIN_SHAPE.fullmatch(origin)
    if not match:
        return False
    scheme, host, port = match.groups()
    if port is not None and (int(port) > 65535 or int(port) == _DEFAULT_PORTS[scheme]):
        return False
    labels = host.split(".")
    try:
        if host.startswith("["):
            inner = host[1:-1]
            return _serialize_ipv6(ipaddress.IPv6Address(inner)) == inner
        if _NUMERIC_LABEL.fullmatch(labels[-1]):
            return str(ipaddress.IPv4Address(host)) == host
    except ValueError:
        return False
    return all(_HOST_LABEL.fullmatch(label) for label in labels)


# Origins that must never be combined with credentialed CORS: "*" admits every
# site, and "null" is what sandboxed iframes and file:// pages send.
_REFUSED_ORIGINS = ("*", "null")


def validate_cors_origins(origins: Iterable[str]) -> list[str]:
    """
    Return the CORS origins trimmed, lower-cased, and without blank entries.

    :raises ServerError: if ``origins`` is a single string rather than a
        list, if an origin is "*" or "null", which credentialed CORS must
        never admit, or if it is not written the way browsers serialize the
        Origin header (see _is_serialized_origin), so it could never match.
    """
    if isinstance(origins, str):
        raise ServerError(
            "cors_allow_origins must be a list of origins, not a single "
            f"string: {origins!r}"
        )
    cleaned = [origin.strip().lower() for origin in origins if origin.strip()]
    for origin in cleaned:
        if origin in _REFUSED_ORIGINS:
            raise ServerError(
                f"cors_allow_origins must not contain '{origin}': the REST "
                "server allows credentials on CORS requests, and that origin "
                "would let pages you do not control make credentialed calls. "
                "List the exact browser origins "
                "(OMNI_EVENTS_CORS_ALLOW_ORIGINS), or leave it empty to "
                "disable CORS."
            )
        if not _is_serialized_origin(origin):
            raise ServerError(
                f"cors_allow_origins entry '{origin}' is not written the way "
                "browsers send an Origin header: use http(s)://host[:port] "
                "with an "
                "ASCII host (punycode for IDNs), no default port (:80 for "
                "http, :443 for https), and no path or trailing slash, for "
                "example https://app.example.com."
            )
    return cleaned


class CommonSettings(NotificationBaseSettings):
    """
    Settings for the App.
    NOTE: all environment variables will be prefixed
          with "OMNI_EVENTS_"
    """

    rabbitmq_uri: str = "amqp://127.0.0.1:5672"
    rabbitmq_channel_pool_size: int = 100
    rabbitmq_channel_pool_min_size: int = 75
    # NON-DURABLE pool packing target and per-consumer prefetch (SDD SR-1.2 /
    # SR-1.3). rabbitmq_channel_client_limit is the pool's packing *target*, not
    # a hard per-channel bound: on an overflow-enabled pool a checkout arriving
    # once every channel is open and full is admitted onto the least-loaded open
    # channel, so client_limit sets the soft packing threshold
    # (pool_size * client_limit = 100 * 8 = 800 streams/pod) rather than a
    # ceiling. The non-durable pool deliberately carries NO per-pool
    # ChannelPoolConfig override — the consuming service's memory-safe stream cap
    # couples to these process-wide globals (see the coupling contract in
    # app/lib/rabbitmq/stream_memory_cap.py), so handing the pool its own config
    # object would break that coupling. rabbitmq_channel_prefetch_count is the
    # dominant broker-memory lever: at 15 a saturated stream costs ~32 kB of
    # broker memory versus ~883 kB at 500.
    rabbitmq_channel_prefetch_count: int = 15
    rabbitmq_channel_wait_ms: int = 500
    rabbitmq_channel_client_limit: int = 8
    # SR-1.5 two-tier admission: governs the NON-DURABLE pool's overflow band,
    # where a checkout that arrives once every channel is open and full is
    # admitted past rabbitmq_channel_client_limit onto the least-loaded open
    # channel instead of being refused. The durable pool ignores this flag by
    # construction (DurableChannelPoolConfig.overflow_enabled is hard-off), and
    # there is deliberately no rabbitmq_durable_channel_overflow_enabled twin.
    # Defaults True: the two-tier acceptance test has to hold at today's
    # shipped defaults, and the service's hard refusal moves to the memory-safe
    # stream cap, so a default-off pool would keep refusing at the pool bound
    # while the capacity guard still admits.
    rabbitmq_channel_overflow_enabled: bool = True
    # Durable / DLQ-browse channel pool (see rabbitmq/channel_pool.py).
    # min_size 0 leaves the pool empty until a durable checkout needs it.
    rabbitmq_durable_channel_pool_size: int = 100
    rabbitmq_durable_channel_pool_min_size: int = 0
    rabbitmq_durable_channel_prefetch_count: int = 500
    rabbitmq_durable_channel_wait_ms: int = 500
    rabbitmq_durable_channel_client_limit: int = 1
    redis_uri: str | None = None
    log_level: str = "INFO"
    grpc_health_checking_enabled: bool = True
    grpc_reflection_enabled: bool = True
    grpc_capabilities_enabled: bool = True
    grpc_keepalive_time_ms: int = 30000  # Send keepalive ping every 30s
    grpc_keepalive_timeout_ms: int = 10000  # Wait 10 seconds for ping ack
    grpc_keepalive_permit_without_calls: bool = True  # Allow pings w/o calls
    rest_port: int = 8000
    # Browser origins the REST server's CORS middleware admits, each answered
    # with Access-Control-Allow-Credentials: true. Empty (the default) adds no
    # CORS middleware at all, so no cross-origin browser request is granted.
    # The environment variable takes a comma-separated list or a JSON array.
    # Each entry is checked by validate_cors_origins when the settings load:
    # "*" and "null" are refused, because combined with credentials they let
    # pages the operator does not control make credentialed calls, and so is
    # anything that is not a bare scheme://host[:port] origin.
    cors_allow_origins: Annotated[list[str], NoDecode] = Field(default_factory=list)

    @field_validator("cors_allow_origins", mode="before")
    @classmethod
    def _parse_cors_allow_origins(cls, value: Any) -> Any:
        """Accept a comma-separated string or a JSON array from the env."""
        if not isinstance(value, str):
            return value
        text = value.strip()
        if text.startswith("["):
            return json.loads(text)
        return text.split(",")

    @field_validator("cors_allow_origins")
    @classmethod
    def _validate_cors_allow_origins(cls, value: list[str]) -> list[str]:
        """Trim, normalize, and refuse unsafe or malformed origins."""
        return validate_cors_origins(value)

    # Endpoint-settle delay for shutdown. On SIGTERM the process first keeps
    # serving normally -- readiness stays SERVING and the capacity guard still
    # admits new streams -- for shutdown_settle_delay_seconds, which buys the
    # routers and endpoint controllers time to converge on the pod's removal
    # before any request is refused. Only then does the drain below begin. The
    # default is 0 (the settle is skipped entirely) because the right value is a
    # routing-propagation property of the cluster, not of the library, and
    # because a non-zero library default would stack on top of a chart that
    # still renders its own preStop sleep and push the total past the
    # termination-grace-period guard. Like the drain settings, this is expected
    # to be templated per deployment via helm.
    shutdown_settle_delay_seconds: float = Field(default=0.0, ge=0)

    # Graceful-drain window for shutdown (SDD SR-6.1 / PRD FR-8, AC-4). On
    # SIGTERM the connector enters a distinct "draining" state and waits up to
    # shutdown_drain_window_seconds for the non-durable pool's in-flight
    # checkouts to reach zero, re-reading the count every
    # shutdown_drain_poll_interval_seconds. The code defaults are safe/non-zero;
    # the deployed window is sized to the proxy streaming-timeout envelope via
    # helm. The poll interval is constrained strictly positive (gt=0): a
    # non-positive interval turns the drain loop into a tight busy-loop for the
    # whole window, so it is rejected at settings-load time.
    shutdown_drain_window_seconds: float = Field(default=30.0, ge=0)
    shutdown_drain_poll_interval_seconds: float = Field(default=0.5, gt=0)

    permissions_endpoint: str | None = None
    permissions_enabled: bool = False
    permissions_ttl_seconds: int = 600

    # observability settings
    #
    # `telemetry_enabled` is the legacy master switch that gates every signal
    # together. `metrics_enabled` and `tracing_enabled` allow the metrics and
    # tracing signals to be enabled independently; when either is left unset
    # (None) it falls back to `telemetry_enabled` (see the validator below), so
    # deployments that only set OMNI_EVENTS_TELEMETRY_ENABLED keep their existing
    # behavior. Log export has no independent switch and follows tracing.
    telemetry_enabled: bool = False
    metrics_enabled: bool | None = None
    tracing_enabled: bool | None = None
    otlp_tracing_endpoint: str = "localhost:4317"
    otlp_metrics_endpoint: str = "localhost:4317"
    otlp_logs_endpoint: str = "localhost:4317"

    @model_validator(mode="after")
    def _default_signal_flags_to_telemetry(self) -> "CommonSettings":
        """Fall back to the legacy master switch for any unset signal flag."""
        if self.metrics_enabled is None:
            self.metrics_enabled = self.telemetry_enabled
        if self.tracing_enabled is None:
            self.tracing_enabled = self.telemetry_enabled
        return self

    # routing key encoding (see notification_common.rabbitmq.routing).
    # RabbitMQ caps routing/binding keys at 255 bytes. The encoding budget
    # is split between event_type and resource_id:
    #   - event_type is capped at routing_max_event_type_parts; longer
    #     event_types raise BadRequestError up front with a clear message.
    #   - resource_id is capped at routing_max_prefix_depth total hashed
    #     segments. Deep paths are restructured into (depth-2) head
    #     segments + 1 folded "middle" hash + 1 separate "last" hash, so
    #     resource paths can be any depth without exceeding the cap. The
    #     last-segment-separate slot is what lets LAZY (".*") filter
    #     bindings line up with the publisher's last hash at any depth.
    # With the default settings these two limits together fit comfortably
    # within 255 bytes regardless of the actual path depth, so publishers
    # using valid event_types will not see runtime "routing key too long"
    # errors. Hash lengths are capped at 32 because hash_string slices an
    # MD5 hex digest (32 chars); higher values would silently degrade.
    routing_hash_length: int = Field(default=5, ge=1, le=32)
    routing_event_type_hash_length: int = Field(default=5, ge=1, le=32)
    # ge=2 because the head/middle/last encoding needs at least the
    # middle and last slots (head can be empty at the minimum).
    routing_max_prefix_depth: int = Field(default=37, ge=2)
    routing_max_event_type_parts: int = Field(default=5, ge=1)

    # only used in tests:
    # Gets set to "tests" by Dockerfile when building for test
    app_env: str = "production"


common_settings = CommonSettings()
