# Changelog

See [keepachangelog](https://keepachangelog.com/en/1.1.0/). Please wrap all
lines to 80 characters using your IDE (e.g. the VSCode [rewrap
extension](https://stkb.github.io/Rewrap/)).

## [Unreleased]

### Removed

* Removed the deprecated gRPC server OTel metric layer. Use the A66 metric layer
  instead.

### Changed

* gRPC A66 metric layers now report the message sizes without gRPC framing
  bytes.

## [0.14.0] - 2026-05-01

### Added

#### HTTP/axum

* The HTTP server metric layer now emits `http.server.request.started`,
  equivalent to `grpc.server.call.started`.
* The HTTP server metric layer now emits the `error.type` attribute.
* `http.server.response.body.size` includes the `http.response.status_code`
  attribute.
* The HTTP trace spans emit `otel.status_description` (span status
  message/description) for error responses (not error status codes).
* The HTTP server trace spans emit `server.address`.

#### gRPC/tonic

* The gRPC trace spans emit the `error.type` attribute.
* The gRPC trace spans emit `otel.status_description` (span status
  message/description) for error responses.

### Changed

#### HTTP/axum

* The HTTP layers track v1.40.0 of the OTel semconv. The following changes are a
  result of that.
* The HTTP server layers now record `network.protocol.version` as e.g. `1.0`,
  `2`, `3` instead of `HTTP/1.1`, `HTTP/2.0`, `HTTP/3.0`, per OTel semconv.
* The HTTP server metric `http.server.request.duration` now matches the trace
  span duration.
* The HTTP server metrics
  `http.server.request.body.size`/`http.server.response.body.size` now
  accumulate the total body size, instead of emitting the metric per data frame.
* The HTTP server trace span is now named `HTTP` and not `_OTHER` for
  non-standard methods. Additionally, the `http.request.method_original`
  attribute is emitted.
* The HTTP server trace span now emits `error.type` for server error status
  codes, not only response errors.
* The HTTP server trace span now emit the `http.route` and `url.path`
  attributes.
* The HTTP server trace layer now emits exception information as an
  `http.server.request.exception` event instead of span attributes, per OTel
  semconv.

#### gRPC/tonic

* The gRPC server OTel metric layer is deprecated. The Omniverse Metrics SRD
  mandates gRPC/A66-style metrics. Additionally, the A66 proposal is more stable
  than the OTel semantic conventions.
* The unit for `grpc.client.attempt.started` was wrong and has been changed from
  `{call}` to `{attempt}`.
* For unknown methods, gRPC server A66 metric layer records `grpc.method` as
  `"other"` to limit cardinality, but only if an `RpcMap` is configured. You are
  strongly encouraged to set an RPC map when building the layer.
* `with_target_fn` functions must now return an `Option<StringValue>` instead of
  a `StringValue`.

* The gRPC trace layer tracks v1.40.0 of the OTel semconv. The following changes
  are a result of that.
* The gRPC trace spans now emit `rpc.response.status_code` (string) instead of
  `rpc.grpc.status_code` (numeric), per OTel semconv.
* The gRPC trace spans now emit `rpc.system.name` instead of `rpc.system`, per
  OTel semconv.
* The gRPC trace spans now emit a fully-qualified `rpc.method` (e.g.
  `example.Unary/Echo`) instead of separate `rpc.service` and `rpc.method`
  attributes, per OTel semconv.
* The gRPC server trace span no longer emits `user_agent.original`, per OTel
  semconv.
* The gRPC client trace layer now emits exception information as an
  `rpc.client.call.exception` event instead of span attributes, per OTel
  semconv.
* The gRPC server trace layer now emits exception information as an
  `rpc.server.call.exception` event instead of span attributes, per OTel
  semconv.
* For unknown methods, gRPC server trace layer records `rpc.method` as
  `"_OTHER"` to limit cardinality. You are strongly encouraged to set an RPC map
  when building the layer. In this case, `rpc.method_original` will contain the
  attacker-controlled, request URI-derived method.
* For unknown methods, gRPC client trace layer records `rpc.method` as
  `"_OTHER"` to limit cardinality, and emits `rpc.method_original`. This should
  not be hit for properly constructed clients.

### Removed

* Removed deprecated `tonic_observability::client::TonicTraceLayer` re-export,
  import from `tonic_observability::client::trace::TonicTraceLayer` instead.
* Removed deprecated `tonic_observability::server::TonicTraceLayer` re-export,
  import from `tonic_observability::server::trace::TonicTraceLayer` instead.
* Removed deprecated `tonic_observability::server::TonicMetricLayer` re-export,
  import from `tonic_observability::server::metric_otel::TonicMetricLayer`
  instead.
* Removed deprecated `rpc_map!()` macro. Use `RpcMap::builder()` to construct an
  RPC map from file descriptor sets.
* Removed deprecated `RpcMap::methods` method. There is no replacement.

## [0.13.0] - 2026-04-03

### Added

* `RpcMap`s can now be constructed from compiled Protobuf file descriptor sets,
  similar to the reflection service.
* Trace layer spans can now be customized. This allows setting the span's target
  name used by `tracing` for e.g. filtering. The code file path/code module name
  corresponds to the definition site when using custom span creation.
* gRPC client metric layer following the A66 spec.

### Changed

* The `rpc_map!` macro is deprecated; use `RpcMap::builder()` to construct an
  RPC map from file descriptor sets. RPC maps constructed this way don't have a
  risk of drifting from the service definition, and can easily include
  additional services such as a health or reflection service.
* The `RpcMap::methods()` function is deprecated; use `RpcMap::builder()` to
  construct an RPC map from file descriptor sets.
* The gRPC client trace layer now emits `server.address`. You should ensure that
  clients are built with an origin, or the layer is built using
  `with_target_fn()`.

## [0.12.0] - 2026-02-18

### Added

* Tonic server metric layer with Google/A66-compatible metric names.
* Axum server trace and metric layer.

### Changed

* Deprecated importing layers and types from
  `tonic_observability::{client,server}`. This is to allow for different metric
  layers.
  * Import `tonic_observability::client::trace::TonicTraceLayer` instead.
  * Import `tonic_observability::server::trace::TonicTraceLayer` instead.
  * Import `tonic_observability::server::metric_otel::TonicMetricLayer` instead.

## [0.11.0] - 2025-11-21

### Changed

* Rust version 1.91.0.
* Allow running the `otel-asserts` collectors on an existing runtime.
* `MetricsGuard::meter()` now returns an `Option<Meter>`, to allow consumers to
  distinguish whether metrics are enabled and to align with
  `LogsGuard`/`TracesGuard`. To get the previous behavior where a no-op meter
  was returned, use `MetricsGuard::meter_or_noop()`.
* The tracer returned by `TracesGuard::tracer()` can now be boxed.
* All guards can create a shutdown handle, to shut down the providers manually
  if the program does not unwind.
* Suppress `target` attribute for spans again. This was included [by default in
  `tracing-opentelemetry`](https://github.com/tokio-rs/tracing-opentelemetry/pull/222),
  and shipped in 0.10.0, since we didn't know how to disable it. To decrease
  span attribute cardinality, this attribute is disabled again.
  `code.module.name` provides this information in the default case, `otel.name`
  can be used to override the span name for OpenTelemetry specifically, and we
  consider this to be a `tracing` implementation detail for filtering.

## [0.10.0] - 2025-11-03

### Changed

* OpenTelemetry 0.31.0. **WARNING**: Many potentially breaking changes, see
  below for some of the major changes. Consult the relevant crates' change logs
  for more information.
* Due to changes in `tracing-opentelemetry`, it is no longer possible to set a
  `Span`'s parent after the span has been "built". This happens when the span is
  accessed, e.g. when it is entered, it's context is retrieved, or when it is
  cloned (!). For more information, see [`tracing-opentelemetry`
  #202](https://github.com/tokio-rs/tracing-opentelemetry/pull/202) and
  [`opentelemetry-rust`
  #2420](https://github.com/open-telemetry/opentelemetry-rust/issues/2420). This
  means you must be very careful when using `set_span_context`. Also,
  `set_span_context` returns a boolean instead of a specific error, because the
  `tracing-opentelemetry` folks [forgot to export the
  type](https://github.com/tokio-rs/tracing-opentelemetry/issues/236). This
  should be fixed in a later version; as a work-around I've marked the return
  type of `set_span_context` as `#[must_use]` for now.
* Due to changes in `tracing-opentelemetry`, exported OpenTelemetry spans now
  have the `target` attribute attached. This value corresponds to the `tracing`
  target. It isn't useful, as we have to specify it at compile time. Although
  it's set to either `client` or `server`, this is an implementation detail and
  should never be used. The [OpenTelemetry span
  kind](https://opentelemetry.io/docs/concepts/signals/traces/#span-kind) is the
  canonical source of this information. You may wish to block this attribute in
  OpenTelemetry collectors.
* The Rust crates now use the semantic conventions for `code.*` stabilized in
  [OpenTelemetry semantic conventions
  v1.33.0](https://github.com/open-telemetry/semantic-conventions/releases/tag/v1.33.0)
  (i.e. the language-independent spec). See this [non-normative migration
  guide](https://opentelemetry.io/docs/specs/semconv/non-normative/code-attrs-migration/).
  Specifically relevant for the layers are that `code.filepath` was changed to
  `code.file.path`, and `code.namespace` was changed to `code.module.name`.

## [0.9.0] - 2025-09-18

### Changed

* Rust version 1.90.0.

## [0.8.0] - 2025-08-08

### Changed

* Rust version 1.89.0.

## [0.7.0] - 2025-07-21

### Changed

* Added optional `serde::Serialize` and `serde::Deserialize` trait
  implementations for various data structures in the `otel-asserts` crate,
  wrapped behind a new `with-serde` feature flag (useful for generating "golden"
  test data to perform checks against).
* Modified the `otel_asserts::grpc::Collector` implementation to take an
  optional worker threads override count when creating a multi-threaded tokio
  runtime to execute a future on.

## [0.6.1] - 2025-07-10

### Changed

* Make `TracingSubscriberBuilder::build_subscriber` a public method.

## [0.6.0] - 2025-06-30

### Changed

* Rust version 1.88.0.

## [0.5.0] - 2025-06-30

### Changed

* OpenTelemetry 0.30.0.
* Tracing layer attribute `exception.escaped` is deprecated per OpenTelemetry
  semantic conventions, and is no longer emitted.

## [0.4.0] - 2025-06-30

### Changed

* **Breaking**: The server metrics layer no longer records header values in
  response sizes. In HTTP/2, headers can be compressed via HPACK, and we don't
  have access to the compressed data in the layer. So the recorded size will be
  inaccurate (sometimes wildly so). Note that some header values like the gRPC
  status message and details are still counted.
* Rust version 1.86.0.

## [0.3.0] - 2025-04-11

### Changed

* The gRPC client trace layer no longer emits the `server.address` and
  `user_agent.original` fields, as these aren't set when the layer is invoked.
* Tonic 0.13 support.
* Rust version 1.85.1.
* OTLP log export now includes the trace and span ID from the `tracing` span.
  Note that the trace flags will always be empty (i.e. not sampled) due to a
  limitation with the OpenTelemetry library.

## [0.2.0] - 2025-03-25

### Changed

* OpenTelemetry 0.29.0 (from 0.27.1).
* Rust version 1.85.0 and Rust edition 2024.
* `OTEL_LOGS_EXPORTER` default is now `otlp,console` instead of `otlp`, to get
  console logging by default.

### Removed

* Removed the `init` method for OTLP exporters.

## [0.1.1] - 2025-03-13

### Added

* Tracing subscriber setup.

### Changed

* Deprecated the `init` method for OTLP exporters. Instead, the exporters/
  guards should be constructed using the `guard()` builder on the respective
  configs.
* Send + Sync bounds on the type-erased return values of `LogsGuard` and
  `TracesGuard`.

## [0.1.0-rc4] - 2025-03-03

No external code changes, ignore.

## [0.1.0-rc3] - 2025-02-12

### Added

* OTLP logs exporter setup
* Re-exported `opentelemetry_sdk::Resource`. Weirdly enough, e.g. `KeyValue` is
  in the `opentelemetry` crate.

### Changed

* `TracesGuard::tracer()` now returns an opaque tracer type.
* Metric `rpc.server.errors` was replaced by `rpc.server.calls`, which also has
  the `rpc.grpc.status_code` attribute attached. This should make dashboard
  queries easier to construct.

## [0.1.0-rc2] - 2025-02-10

### Added

* `ResourceBuilder` to simplify Otel resource construction
* OTLP traces exporter setup
* OTLP metrics exporter setup

### Changed

* Implement `std::error::Error` for `TraceParentError`

## [0.1.0-rc1] - 2025-02-04

### Added

* Initial release of `tonic-observability` and `otel-tools`
