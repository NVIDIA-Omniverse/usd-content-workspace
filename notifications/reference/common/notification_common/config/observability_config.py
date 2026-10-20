# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import logging

import opentelemetry.exporter.otlp.proto.grpc._log_exporter as logs_exporter
import opentelemetry.exporter.otlp.proto.grpc.metric_exporter as metrics_exporter
import opentelemetry.exporter.otlp.proto.grpc.trace_exporter as trace_exporter
import opentelemetry.sdk._logs as otel_sdk_logs
import opentelemetry.sdk._logs.export as otel_sdk_logs_export
import opentelemetry.sdk.metrics as otel_sdk_metrics
import opentelemetry.sdk.metrics.export as otel_sdk_metrics_export
import opentelemetry.sdk.trace as otel_sdk_trace
import opentelemetry.sdk.trace.export as otel_sdk_trace_export
from opentelemetry import _logs
from opentelemetry import metrics
from opentelemetry import trace
from opentelemetry.instrumentation import aio_pika as aio_pika_instrumentor
from opentelemetry.instrumentation import logging as logging_instrumentor
from opentelemetry.instrumentation.grpc.filters import (
    any_of,
    negate,
    health_check,
    service_name,
)
from opentelemetry.sdk.resources import Resource

from nvidia.omniverse.discovery.capabilities.v2alpha import capabilities_service_pb2

from notification_common.config.common_settings import common_settings
from notification_common.config.metadata import project_name

# Full name of the Discovery CapabilitiesService, derived from the generated
# descriptor so this is the single source of truth for the service name across
# the auth/metrics exemptions, the tracing filter, and the reflection/health
# wiring in notification_server (which imports CAPABILITIES_SERVICE_NAME from
# here). Deriving it from the descriptor keeps those sites from drifting.
CAPABILITIES_SERVICE_NAME = capabilities_service_pb2.DESCRIPTOR.services_by_name[
    "CapabilitiesService"
].full_name
_CAPABILITIES_LIST_SERVICES_METHOD = f"/{CAPABILITIES_SERVICE_NAME}/ListServices"

# Methods that should be excluded from observability instrumentation.
#
# NOTE: this tuple has THREE consumers and doubles as an auth-exemption list,
# so edits here change auth behavior:
#   - AuthInterceptor (interceptors/auth_interceptor.py) skips authentication
#     for these methods, keeping them unauthenticated.
#   - ServerMetricsInterceptor (interceptors/metrics/server_metrics_interceptor.py)
#     skips metrics instrumentation for these methods.
#   - The tracing exclusion uses the separate service-name filter below
#     (create_instrumentation_exclude_filter), not this tuple.
# The Discovery CapabilitiesService ListServices RPC is unauthenticated by
# design so the discovery scan can reach it without credentials.
EXCLUDE_METHODS = (
    "/grpc.health.v1.Health/Check",
    "/grpc.health.v1.Health/Watch",
    "/grpc.reflection.v1alpha.ServerReflection/ServerReflectionInfo",
    _CAPABILITIES_LIST_SERVICES_METHOD,
)


def create_instrumentation_exclude_filter():
    """Create a filter that excludes health, reflection, and capabilities."""
    return negate(
        any_of(
            health_check(),
            service_name("grpc.reflection.v1alpha.ServerReflection"),
            service_name(CAPABILITIES_SERVICE_NAME),
        )
    )


def configure_observability():
    """
    Instruments otel tracing, metrics, and logging
    """
    if not (common_settings.tracing_enabled or common_settings.metrics_enabled):
        return

    resource = Resource.create({"service.name": project_name()})

    if common_settings.tracing_enabled:
        _setup_tracing(resource)
    if common_settings.metrics_enabled:
        _setup_metrics(resource)

    # Log export has no independent switch; ship logs whenever any signal is
    # on (we already returned above if neither is). Trace/span-id correlation
    # in the logs still requires tracing (see _instrument_dependencies).
    _setup_logging(resource)

    if common_settings.tracing_enabled:
        # aio_pika spans and log/trace-id correlation are tracing concerns.
        _instrument_dependencies()


_OTLP_EXPORT_TIMEOUT_SECONDS = 2
"""
Per-export timeout for OTLP gRPC calls (default would be 10s).

Tightened so a wedged or rebounding otelcol agent cannot block our
event loop long enough for kubelet's 1s gRPC liveness probe to fail
3 times in a row. Telemetry export blocking the loop has caused
repeated pod-kills in dev when the local agent was unhealthy.

Tracing/metrics/logs are best-effort by design — drop fast rather
than hold up request handling.
"""


def _setup_tracing(service_resource: Resource) -> None:
    trace_provider = otel_sdk_trace.TracerProvider(resource=service_resource)
    trace_provider.add_span_processor(
        otel_sdk_trace_export.BatchSpanProcessor(
            trace_exporter.OTLPSpanExporter(
                endpoint=common_settings.otlp_tracing_endpoint,
                insecure=True,
                timeout=_OTLP_EXPORT_TIMEOUT_SECONDS,
            )
        )
    )

    trace.set_tracer_provider(trace_provider)


def _setup_metrics(service_resource: Resource) -> None:
    metric_readers = [
        otel_sdk_metrics_export.PeriodicExportingMetricReader(
            metrics_exporter.OTLPMetricExporter(
                endpoint=common_settings.otlp_metrics_endpoint,
                insecure=True,
                timeout=_OTLP_EXPORT_TIMEOUT_SECONDS,
            )
        )
    ]

    meter_provider = otel_sdk_metrics.MeterProvider(
        resource=service_resource, metric_readers=metric_readers
    )

    metrics.set_meter_provider(meter_provider)


def _setup_logging(service_resource: Resource) -> None:
    logger_provider = otel_sdk_logs.LoggerProvider(resource=service_resource)

    log_exporter = logs_exporter.OTLPLogExporter(
        endpoint=common_settings.otlp_logs_endpoint,
        insecure=True,
        timeout=_OTLP_EXPORT_TIMEOUT_SECONDS,
    )

    logger_provider.add_log_record_processor(
        otel_sdk_logs_export.BatchLogRecordProcessor(log_exporter)
    )

    _logs.set_logger_provider(logger_provider)

    handler = otel_sdk_logs.LoggingHandler(logger_provider=logger_provider)
    logging.getLogger().addHandler(handler)


def _instrument_dependencies() -> None:
    logging_instrumentor.LoggingInstrumentor().instrument()
    aio_pika_instrumentor.AioPikaInstrumentor().instrument()
