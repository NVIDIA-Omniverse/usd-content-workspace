# gRPC client spans

See [semantic conventions for RPC spans](otel-rpc-spans), [RPC exceptions](otel-rpc-exceptions), [gRPC](otel-rpc-grpc), and [recording errors](otel-record-errors). Tracking [v1.40.0](../spec/otel-rpc-spans.md)/[v1.40.0](../spec/otel-rpc-exceptions.md)/[v1.40.0](../spec/otel-grpc.md)/[v1.40.0](../spec/otel-recording-errors.md).

[otel-rpc-spans]: https://opentelemetry.io/docs/specs/semconv/rpc/rpc-spans/
[otel-rpc-exceptions]: https://opentelemetry.io/docs/specs/semconv/rpc/rpc-exceptions/
[otel-rpc-grpc]: https://opentelemetry.io/docs/specs/semconv/rpc/grpc/
[otel-record-errors]: https://opentelemetry.io/docs/specs/semconv/general/recording-errors/

## Contents

- [Context propagation](#context-propagation)
- [`tracing` span name and location](#tracing-span-name-and-location)
- [Span attributes](#span-attributes)
- [Exception events](#exception-events)

## Context propagation

This layer injects the span's context into outgoing requests using W3C Trace Context headers (`traceparent`, `tracestate`).

## `tracing` span name and location

See [the shared layer documentation](./grpc-layers.md#tracing-span-name-and-location).

## Span attributes

Standard `tracing` attributes may be recorded, depending on the `tracing_opentelemetry::OpenTelemetryLayer` configuration: `busy_ns`, `code.file.path`, `code.line.number`, `code.module.name`, `idle_ns`, `thread.id`, `thread.name`, `target`.

| OTel Attribute                                |    | Note |
| ---                                           | -- | - |
| `otel.status_code`                            | ✅ | 1 |
| `otel.status_description`                     | ✅ | 1 |
| `error.type` (stable)                         | ✅ | 2 |
| `network.peer.address` (stable)               | ❌ | 6 |
| `network.peer.port` (stable)                  | ❌ | 6 |
| `rpc.method_original` (rc)                    | ✅ | 3 |
| `rpc.method` (rc)                             | ✅ | 3 |
| `rpc.request.metadata.<key>` (development)    | ❌ | 4 |
| `rpc.response.metadata.<key>` (development)   | ❌ | 4 |
| `rpc.response.status_code` (rc)               | ✅ |   |
| `rpc.system.name` (rc)                        | ✅ |   |
| `server.address` (stable)                     | ✅ | 5 |
| `server.port` (stable)                        | ❌ | 5 |

Notes:

1. `otel.status_code`: Set to `"ERROR"` for gRPC status codes considered a client-side error and transport errors. If available, `otel.status_description` will contain more information.
2. `error.type`: Matches `rpc.response.status_code` for client-side errors, otherwise omitted. Rust doesn't have exceptions; tonic always returns a code. It's redundant but recorded for OTel semconv adherence.
3. `rpc.method`/`rpc.method_original`: To limit attribute cardinality for samplers, if the tonic `GrpcMethod` extension is absent from the request, `rpc.method` is set to `"_OTHER"` and `rpc.method_original` is recorded from the request URI path.
4. `rpc.request.metadata.<key>`, `rpc.response.metadata.<key>`: Not recorded automatically to avoid capturing sensitive information.
5. `server.address`/`server.port`: `server.port` is not recorded. If available, the full target is recorded in `server.address` to match the gRPC A66 proposal/metrics. This is a semconv violation. See the shared layer documentation [on how to specify target/address values](./grpc-layers.md). If no target can be derived, this attribute is omitted.
6. `network.peer.*`: Unavailable to the layer; `server.address` provides the gRPC target.

## Exception events

When a transport error occurs, or a response body returns a status with a source set, the layer emits an event per the [semantic conventions RPC exceptions](otel-rpc-exceptions).

The event is always named `rpc.client.call.exception`, is emitted at the WARN level, and includes the following attributes:

* `exception.type`: The Rust type name of the error.
* `exception.message`: The status message or `Display` representation of the error.
* `exception.stacktrace`: This representation isn't stable. It's intended to give a rough idea of the error context. Currently, the error chain is printed using the `Display` representation of the error and its sources. We may use [`std::error::Report`](https://doc.rust-lang.org/std/error/struct.Report.html) or similar in the future.

Note: If the backend doesn't collect events for traces/spans, this information can be lost. Also, because the trace layer uses `tracing`, these events may show up in logs depending on how that's configured.
