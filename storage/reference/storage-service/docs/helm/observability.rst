Observability
=============

The chart configures logging, metrics, and tracing for Storage Service. Metrics can be exposed for Prometheus to scrape or pushed to an OpenTelemetry Protocol (OTLP) collector. Traces are pushed to an OTLP collector. OTLP uses gRPC transport.

Metrics
-------

Choose one metric delivery mode:

* Set ``service.metrics.enabled`` to ``true`` to expose a Prometheus endpoint on ``service.metrics.port``.
* Set ``observability.metrics.enabled`` to ``true`` to push metrics to the configured OTLP collector. OTLP metrics take precedence when both settings are enabled, so the Prometheus endpoint is not used as the metrics exporter.

The chart exposes the Prometheus port through both Service resources only when ``service.metrics.enabled`` is ``true``. Configure the associated Service annotations, such as Prometheus scrape annotations, with ``service.annotations`` or ``podAnnotations``.

``service.metrics.enabled``
  :Required: no
  :Type: boolean
  :Default: ``false``

  Enables Prometheus exposition. Ignored as an exporter when OTLP metrics are enabled.

``service.metrics.port``
  :Required: no
  :Type: integer
  :Default: ``8013``

  Port for the Prometheus metrics endpoint.

``config.metrics.prefix``
  :Required: no
  :Type: string
  :Default: ``storage``

  Prefix for metric names. Set to an empty string to emit unprefixed names.

``observability.metrics.enabled``
  :Required: no
  :Type: boolean
  :Default: ``false``

  Enables OTLP metric export.

``observability.metrics.endpoint.grpc``
  :Required: when ``observability.metrics.enabled`` is set to ``true``
  :Type: string

  gRPC endpoint of the OTLP-compatible collector.

``observability.metrics.exportIntervalMs``
  :Required: no
  :Type: integer
  :Default: ``15000``

  Interval, in milliseconds, between OTLP metric exports.

Prometheus scraping configuration
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

Enable the Prometheus exporter and add scrape annotations to the pod. Prometheus scrapes the ``/metrics`` path on the configured metrics port. Do not enable ``observability.metrics`` in this configuration because OTLP metrics take precedence over the Prometheus exporter.

.. code-block:: yaml

  service:
    metrics:
      enabled: true
      port: 8013
  podAnnotations:
    prometheus.io/scrape: "true"
    prometheus.io/path: "/metrics"
    prometheus.io/port: "8013"

For example, the following configuration pushes metrics to an in-cluster collector:

.. code-block:: yaml

  observability:
    metrics:
      enabled: true
      endpoint:
        grpc: "http://otel-collector.observability.svc.cluster.local:4317"
      exportIntervalMs: 15000

Tracing
-------

Enable tracing to export spans to an OTLP-compatible collector. The collector endpoint is required when tracing is enabled. To use probabilistic sampling, configure ``samplingRatio`` as a string from ``0`` to ``1``. When it is set, the chart configures parent-based trace ID ratio sampling; ``1`` exports all root traces and ``0`` exports none. Parent sampling decisions are preserved.

``observability.tracing.enabled``
  :Required: no
  :Type: boolean
  :Default: ``false``

  Enables OTLP trace export.

``observability.tracing.endpoint.grpc``
  :Required: when ``observability.tracing.enabled`` is set to ``true``
  :Type: string

  gRPC endpoint of the OTLP-compatible collector.

``observability.tracing.samplingRatio``
  :Required: no
  :Type: string

  Optional sampling ratio from ``0`` to ``1``.

``config.additionalHeaders.tracingHeaderNames``
  :Required: no
  :Type: array of strings
  :Default: ``[x-request-id]``

  Request headers to propagate to the storage backends for distributed tracing. This list is independent of ``observability.headers.forward``, which selects the headers forwarded to the permission service. Set a header in both lists to propagate it to every downstream.

.. code-block:: yaml

  observability:
    tracing:
      enabled: true
      endpoint:
        grpc: "http://otel-collector.observability.svc.cluster.local:4317"
      samplingRatio: "0.5"

Header forwarding
-----------------

``observability.headers.forward``
  :Required: no
  :Type: array of strings
  :Default: ``[]``

  Headers of the incoming request to copy onto the permission queries made on its behalf. Names are case-insensitive, and every value of a repeated header is forwarded. Storage Service does not authenticate these values, so unless a trusted proxy sets or strips them the permission service must treat them as caller-controlled input.

  Storage Service refuses to start when the list names a header it sets itself or one the transports treat specially; the error names the offending header.

  This list is independent of ``config.additionalHeaders.tracingHeaderNames``, which propagates headers to the storage backends. Set a header in both lists to propagate it to every downstream.

.. code-block:: yaml

  observability:
    headers:
      forward:
        - "user-id"
        - "username"
        - "username-source-claim"
        - "x-forwarded-for"
        - "x-request-id"

Logging
-------

Enable structured logging to emit one JSON object per log line, which is suitable for log aggregation systems. The ``config.logging`` settings control log filtering and Rust backtraces for the main service container and the Postgres metadata bootstrap container.

``observability.logging.structured.enabled``
  :Required: no
  :Type: boolean
  :Default: ``false``

  Emits logs as JSON instead of human-readable text.

``config.logging.level``
  :Required: no
  :Type: string
  :Default: ``info``

  Log level for Storage Service components. Supported values are ``trace``, ``debug``, ``info``, ``warn``, and ``error``.

``config.logging.backtrace``
  :Required: no
  :Type: string
  :Default: ``0``

  Rust backtrace verbosity. The chart passes this value to the ``RUST_BACKTRACE`` environment variable. Supported values are:

  * ``0`` disables backtraces.
  * ``1`` enables a shortened backtrace that omits internal Rust frames.
  * ``full`` enables a complete backtrace, including internal Rust frames.

``config.logging.extraTargets``
  :Required: no
  :Type: string

  Comma-separated module filters, for example ``hyper=debug,tokio=trace``.

.. code-block:: yaml

  observability:
    logging:
      structured:
        enabled: true
  config:
    logging:
      level: info
      extraTargets: "hyper=warn"
