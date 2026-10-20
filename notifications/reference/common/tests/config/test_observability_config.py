# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import logging
from unittest.mock import MagicMock
import grpc

import opentelemetry.exporter.otlp.proto.grpc._log_exporter as logs_exporter
import opentelemetry.exporter.otlp.proto.grpc.metric_exporter as metrics_exporter
import opentelemetry.exporter.otlp.proto.grpc.trace_exporter as trace_exporter
import opentelemetry.sdk._logs as otel_sdk_logs
import opentelemetry.sdk._logs.export as otel_sdk_logs_export
import opentelemetry.sdk.metrics as otel_sdk_metrics
import opentelemetry.sdk.metrics.export as otel_sdk_metrics_export
import opentelemetry.sdk.trace as otel_sdk_trace
import opentelemetry.sdk.trace.export as otel_sdk_trace_export
import pytest
from opentelemetry import trace, metrics, _logs
from opentelemetry.instrumentation import aio_pika as aio_pika_instrumentor
from opentelemetry.instrumentation import logging as logging_instrumentor
from opentelemetry.sdk.resources import Resource

from notification_common.config import observability_config
from notification_common.config.common_settings import common_settings


@pytest.fixture(autouse=True)
def mock_opentelemetry(mocker):
    """
    Mocking functions that result in otel setup not desired for unit testing
    """
    mocker.patch.object(observability_config, "_setup_tracing")
    mocker.patch.object(observability_config, "_setup_metrics")
    mocker.patch.object(observability_config, "_setup_logging")
    mocker.patch.object(observability_config, "_instrument_dependencies")

    mocker.patch("opentelemetry.trace.set_tracer_provider")
    mocker.patch("opentelemetry.metrics.set_meter_provider")
    mocker.patch("opentelemetry._logs.set_logger_provider")


@pytest.fixture
def mock_resource(mocker):
    mock_resource = MagicMock()
    mocker.patch.object(Resource, "create", return_value=mock_resource)
    return mock_resource


class TestConfigureObservability:
    @pytest.fixture(autouse=True)
    def mock_settings(self):
        initial = (common_settings.metrics_enabled, common_settings.tracing_enabled)
        common_settings.metrics_enabled = True
        common_settings.tracing_enabled = True

        yield

        common_settings.metrics_enabled, common_settings.tracing_enabled = initial

    def test_does_nothing_if_no_signal_enabled(self, mocker):
        spies = [
            mocker.spy(observability_config, "_setup_tracing"),
            mocker.spy(observability_config, "_setup_metrics"),
            mocker.spy(observability_config, "_setup_logging"),
            mocker.spy(observability_config, "_instrument_dependencies"),
        ]

        common_settings.metrics_enabled = False
        common_settings.tracing_enabled = False
        observability_config.configure_observability()

        for spy in spies:
            assert spy.call_count == 0

    def test_metrics_only_ships_logs_but_skips_tracing_and_instrumentation(
        self, mocker
    ):
        common_settings.metrics_enabled = True
        common_settings.tracing_enabled = False
        setup_tracing = mocker.patch.object(observability_config, "_setup_tracing")
        setup_metrics = mocker.patch.object(observability_config, "_setup_metrics")
        setup_logging = mocker.patch.object(observability_config, "_setup_logging")
        instrument = mocker.patch.object(
            observability_config, "_instrument_dependencies"
        )

        observability_config.configure_observability()

        setup_metrics.assert_called_once()
        # Log export follows "any signal on", not tracing specifically.
        setup_logging.assert_called_once()
        setup_tracing.assert_not_called()
        instrument.assert_not_called()

    def test_tracing_only_skips_metrics(self, mocker):
        common_settings.metrics_enabled = False
        common_settings.tracing_enabled = True
        setup_tracing = mocker.patch.object(observability_config, "_setup_tracing")
        setup_metrics = mocker.patch.object(observability_config, "_setup_metrics")
        setup_logging = mocker.patch.object(observability_config, "_setup_logging")
        instrument = mocker.patch.object(
            observability_config, "_instrument_dependencies"
        )

        observability_config.configure_observability()

        setup_tracing.assert_called_once()
        setup_logging.assert_called_once()
        instrument.assert_called_once()
        setup_metrics.assert_not_called()

    def test_resource_instantiation(self, mocker):
        spy = mocker.patch.object(Resource, "create")
        observability_config.configure_observability()

        spy.assert_called_once_with({"service.name": "notification-common"})

    @pytest.mark.parametrize(
        "method",
        ["_setup_tracing", "_setup_metrics", "_setup_logging"],
    )
    def test_setup_methods_called_with_correct_args(
        self, mocker, mock_resource, method
    ):
        mocker.patch.object(
            common_settings, "otlp_tracing_endpoint", "tracing.endpoint"
        )
        mocker.patch.object(
            common_settings, "otlp_metrics_endpoint", "metrics.endpoint"
        )
        mocker.patch.object(common_settings, "otlp_logs_endpoint", "logs.endpoint")

        mock_method = mocker.patch.object(observability_config, method)

        observability_config.configure_observability()

        mock_method.assert_called_once_with(mock_resource)

    def test_calls_instrument_dependencies(self, mocker):
        mock_instrumentors = mocker.patch.object(
            observability_config, "_instrument_dependencies"
        )

        observability_config.configure_observability()

        mock_instrumentors.assert_called_once_with()


class TestSetupTracing:
    @pytest.fixture(autouse=True)
    def unmock_setup_tracing(self, mocker):
        mocker.stop(observability_config._setup_tracing)

    @pytest.fixture
    def mock_tracer_provider(self, mocker):
        mock_tracer_provider = MagicMock()
        mocker.patch.object(
            otel_sdk_trace, "TracerProvider", return_value=mock_tracer_provider
        )
        return mock_tracer_provider

    def setup_tracing(self, mock_resource):
        observability_config._setup_tracing(mock_resource)

    def test_creates_trace_provider_with_the_service_resource(
        self, mocker, mock_resource
    ):
        spy = mocker.patch.object(otel_sdk_trace, "TracerProvider")
        self.setup_tracing(mock_resource)
        spy.assert_called_once_with(resource=mock_resource)

    def test_creates_span_exporter(self, mocker, mock_resource):
        mocker.patch.object(
            common_settings, "otlp_tracing_endpoint", "tracing.endpoint"
        )
        spy = mocker.patch.object(trace_exporter, "OTLPSpanExporter")
        self.setup_tracing(mock_resource)
        spy.assert_called_once_with(
            endpoint="tracing.endpoint", insecure=True, timeout=2
        )

    def test_creates_batch_span_processor(self, mocker, mock_resource):
        mocker.patch.object(trace_exporter, "OTLPSpanExporter", return_value="exporter")
        spy = mocker.patch.object(otel_sdk_trace_export, "BatchSpanProcessor")
        self.setup_tracing(mock_resource)
        spy.assert_called_once_with("exporter")

    def test_adds_batch_span_processor_to_trace_provider(
        self, mock_tracer_provider, mocker, mock_resource
    ):
        mocker.patch.object(
            otel_sdk_trace_export, "BatchSpanProcessor", return_value="processor"
        )
        self.setup_tracing(mock_resource)
        mock_tracer_provider.add_span_processor.assert_called_once_with("processor")

    def test_sets_the_tracer_provider(
        self, mock_tracer_provider, mocker, mock_resource
    ):
        spy = mocker.patch.object(trace, "set_tracer_provider")
        self.setup_tracing(mock_resource)
        spy.assert_called_once_with(mock_tracer_provider)


class TestSetupMetrics:
    @pytest.fixture(autouse=True)
    def unmock_setup_metrics(self, mocker):
        mocker.stop(observability_config._setup_metrics)

    def setup_metrics(self, mock_resource):
        observability_config._setup_metrics(mock_resource)

    def test_creates_metrics_exporter(self, mocker, mock_resource):
        mocker.patch.object(
            common_settings, "otlp_metrics_endpoint", "metrics.endpoint"
        )
        spy = mocker.patch.object(metrics_exporter, "OTLPMetricExporter")
        self.setup_metrics(mock_resource)
        spy.assert_called_once_with(
            endpoint="metrics.endpoint", insecure=True, timeout=2
        )

    def test_creates_exporting_metric_reader(self, mocker, mock_resource):
        mocker.patch.object(
            metrics_exporter, "OTLPMetricExporter", return_value="exporter"
        )
        spy = mocker.patch.object(
            otel_sdk_metrics_export, "PeriodicExportingMetricReader"
        )
        self.setup_metrics(mock_resource)
        spy.assert_called_once_with("exporter")

    def test_creates_meter_provider_with_the_service_resource_and_metric_reader(
        self, mocker, mock_resource
    ):
        spy = mocker.patch.object(otel_sdk_metrics, "MeterProvider")
        mocker.patch.object(
            otel_sdk_metrics_export,
            "PeriodicExportingMetricReader",
            return_value="metric_reader",
        )
        self.setup_metrics(mock_resource)
        spy.assert_called_once_with(
            resource=mock_resource, metric_readers=["metric_reader"]
        )

    def test_sets_the_meter_provider(self, mocker, mock_resource):
        spy = mocker.patch.object(metrics, "set_meter_provider")
        mock_meter_provider = MagicMock()
        mocker.patch.object(
            otel_sdk_metrics, "MeterProvider", return_value=mock_meter_provider
        )

        self.setup_metrics(mock_resource)
        spy.assert_called_once_with(mock_meter_provider)


class TestSetupLogging:
    @pytest.fixture(autouse=True)
    def unmock_setup_logging(self, mocker):
        mocker.stop(observability_config._setup_logging)

    @pytest.fixture
    def mock_logging_provider(self, mocker):
        mock_logging_provider = MagicMock()
        mocker.patch.object(
            otel_sdk_logs, "LoggerProvider", return_value=mock_logging_provider
        )
        return mock_logging_provider

    def setup_logging(self, mock_resource):
        observability_config._setup_logging(mock_resource)

    def test_creates_logging_provider_with_the_service_resource(
        self, mocker, mock_resource
    ):
        spy = mocker.patch.object(otel_sdk_logs, "LoggerProvider")
        self.setup_logging(mock_resource)
        spy.assert_called_once_with(resource=mock_resource)

    def test_creates_logs_exporter(self, mocker, mock_resource):
        mocker.patch.object(common_settings, "otlp_logs_endpoint", "logs.endpoint")
        spy = mocker.patch.object(logs_exporter, "OTLPLogExporter")
        self.setup_logging(mock_resource)
        spy.assert_called_once_with(endpoint="logs.endpoint", insecure=True, timeout=2)

    def test_adds_record_processor_to_logging_provider(
        self, mock_logging_provider, mocker, mock_resource
    ):
        mocker.patch.object(
            otel_sdk_logs_export, "BatchLogRecordProcessor", return_value="processor"
        )
        self.setup_logging(mock_resource)
        mock_logging_provider.add_log_record_processor.assert_called_once_with(
            "processor"
        )

    def test_sets_the_logging_provider(
        self, mock_logging_provider, mocker, mock_resource
    ):
        spy = mocker.patch.object(_logs, "set_logger_provider")
        self.setup_logging(mock_resource)
        spy.assert_called_once_with(mock_logging_provider)

    def test_creates_logging_handler(
        self, mock_logging_provider, mocker, mock_resource
    ):
        spy = mocker.patch.object(otel_sdk_logs, "LoggingHandler")
        self.setup_logging(mock_resource)
        spy.assert_called_once_with(logger_provider=mock_logging_provider)

    def test_adds_logging_handler_to_logger(
        self, mock_logging_provider, mocker, mock_resource
    ):
        mock_logging_handler = MagicMock()
        mocker.patch.object(
            otel_sdk_logs, "LoggingHandler", return_value=mock_logging_handler
        )

        mock_logger = MagicMock()
        mocker.patch.object(logging, "getLogger", return_value=mock_logger)

        self.setup_logging(mock_resource)
        mock_logger.addHandler.assert_called_once_with(mock_logging_handler)


class TestInstrumentDependencies:
    @pytest.fixture(autouse=True)
    def unmock_instrument_dependencies(self, mocker):
        mocker.stop(observability_config._instrument_dependencies)

    def test_instruments_logging(self, mocker):
        mock_instrumentor = MagicMock()
        mocker.patch.object(
            logging_instrumentor,
            "LoggingInstrumentor",
            return_value=mock_instrumentor,
        )
        observability_config._instrument_dependencies()
        mock_instrumentor.instrument.assert_called_once_with()

    def test_instruments_aio_pika(self, mocker):
        mock_instrumentor = MagicMock()
        mocker.patch.object(
            aio_pika_instrumentor,
            "AioPikaInstrumentor",
            return_value=mock_instrumentor,
        )

        observability_config._instrument_dependencies()

        mock_instrumentor.instrument.assert_called_once_with()


class DummyCallDetails(grpc.HandlerCallDetails):
    def __init__(self, method):
        self.method = method
        self.invocation_metadata = None


CAPABILITIES_LIST_SERVICES_METHOD = (
    "/nvidia.omniverse.discovery.capabilities.v2alpha.CapabilitiesService/ListServices"
)


class TestExcludeMethods:
    def test_excludes_capabilities_list_services(self):
        assert CAPABILITIES_LIST_SERVICES_METHOD in observability_config.EXCLUDE_METHODS


class TestCreateInstrumentationExcludeFilter:
    @pytest.mark.parametrize(
        "method_name,should_trace",
        [
            ("/grpc.health.v1.Health/Check", False),
            ("/grpc.health.v1.Health/Watch", False),
            ("/grpc.reflection.v1alpha.ServerReflection/ServerReflectionInfo", False),
            (CAPABILITIES_LIST_SERVICES_METHOD, False),
            ("/notification.NotificationService/SendNotification", True),
            ("/other.Service/Method", True),
        ],
        ids=[
            "health_check",
            "health_watch",
            "reflection",
            "capabilities_list_services",
            "notification_service",
            "other_service",
        ],
    )
    def test_excludes_health_reflection_and_capabilities(
        self, method_name, should_trace
    ):
        filter_func = observability_config.create_instrumentation_exclude_filter()
        call_details = DummyCallDetails(method_name)

        assert filter_func(call_details) == should_trace
