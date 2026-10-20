# HTTP server spans

See [semantic conventions for HTTP spans](otel-http-spans) and [HTTP exceptions](otel-http-exceptions). Tracking [v1.40.0](../spec/otel-http-spans.md)/[v1.40.0](../spec/otel-http-exceptions.md).

[otel-http-spans]: https://opentelemetry.io/docs/specs/semconv/http/http-spans/
[otel-http-exceptions]: https://opentelemetry.io/docs/specs/semconv/http/http-exceptions/

## Contents

- [Context propagation](#context-propagation)
- [`tracing` span name and location](#tracing-span-name-and-location)
- [Span attributes](#span-attributes)
- [Exception events](#exception-events)

## Context propagation

This layer extracts the parent context from incoming requests using W3C Trace Context headers (`traceparent`, `tracestate`).

Response trace context propagation (via the `traceresponse` header) is currently not implemented.

## `tracing` span name and location

By default, the `tracing` span is created in the `axum-observability` crate. This means:

* The trace layer span target name is hardcoded to `server`. The span target is not used by OpenTelemetry. The exported span name is always set to the HTTP method and route (if available). The span target is used by `tracing` to filter traces, for example via `RUST_LOG`.
* The code file path and module name are visible in exported spans, and will point to the `axum-observability` crate.

To customize the span creation, use the `.with_span()` method on the layer builder along with the `make_span!()` macro.

## Span attributes

Standard `tracing` attributes may be recorded, depending on the `tracing_opentelemetry::OpenTelemetryLayer` configuration: `busy_ns`, `code.file.path`, `code.line.number`, `code.module.name`, `idle_ns`, `thread.id`, `thread.name`, `target`.

| OTel Attribute                             |    | Note |
| ---                                        | -- | -- |
| `otel.status_code`                         | ✅ |  2 |
| `otel.status_description`                  | ✅ |  2 |
| `client.address` (stable)                  | ❌ |  3 |
| `client.port` (stable)                     | ❌ |  3 |
| `error.type` (stable)                      | ✅ |  4 |
| `http.request.body.size` (development)     | ❌ |  5 |
| `http.request.header.<key>` (stable)       | ❌ |  6 |
| `http.request.method_original` (stable)    | ⚠️ |  7 |
| `http.request.method` (stable)             | ⚠️ |  7 |
| `http.request.size` (development)          | ❌ |  5 |
| `http.response.body.size` (development)    | ❌ |  5 |
| `http.response.header.<key>` (stable)      | ❌ |  6 |
| `http.response.size` (development)         | ❌ |  5 |
| `http.response.status_code` (stable)       | ✅ |    |
| `http.route` (stable)                      | ✅ |  8 |
| `network.local.address` (stable)           | ❌ |  3 |
| `network.local.port` (stable)              | ❌ |  3 |
| `network.peer.address` (stable)            | ❌ |  3 |
| `network.peer.port` (stable)               | ❌ |  3 |
| `network.protocol.name` (stable)           | ❌ |  9 |
| `network.protocol.version` (stable)        | ✅ |    |
| `network.transport` (stable)               | ❌ |    |
| `server.address` (stable)                  | ✅ | 10 |
| `server.port` (stable)                     | ❌ | 10 |
| `url.path` (stable)                        | ✅ | 11 |
| `url.query` (stable)                       | ❌ | 12 |
| `url.scheme` (stable)                      | ❌ |  9 |
| `user_agent.original` (stable)             | ✅ |    |
| `user_agent.synthetic.type` (development)  | ❌ |    |

Notes:

1. There are a lot of attributes. They all make sense at some level, but quickly bloat the amount of data being sent.
2. `otel.status_code`: If a response body error occurs, set to `"ERROR"` and `otel.status_description` will be set to the error's `Display` representation. Otherwise, if the response status code is a server error (5xx), set to `"ERROR"` and description is not set to avoid duplicating information in `error.type`. For other status codes, this attribute and the description is unset.
3. `client.address`, `client.port`, `network.local.*`, `network.peer.*`: Requires `ConnectInfo<SocketAddr>` which needs opt-in when the server is constructed. Currently not supported.
4. `error.type`: If a response body error occurred, set to the Rust type name of the error. Otherwise, if a server-side error occurred (5xx), matches `http.response.status_code` (i.e. a numeric code). Request body errors should be handled by axum or the handler.
5. As a metrics layer exists, the trace layer doesn't capture metrics.
6. `http.request.header.<key>`, `http.response.header.<key>`: Not recorded automatically to avoid capturing sensitive information.
7. `http.request.method`: Canonicalized/case-insensitive due to `http`/`axum` limitations; `http` does not provide the non-canonicalized value. `OTEL_INSTRUMENTATION_HTTP_KNOWN_METHODS` is not implemented; this is a semconv violation. `http.request.method_original` is only recorded when the method is unknown, not when the method is canonicalized. This is a semconv violation.
8. `http.route`: Only recorded if `axum::extract::MatchedPath` is available.
9. `network.protocol.name`, `url.scheme`: These attributes aren't recorded. This is a deliberate choice and an OTel semconv violation, see [HTTP server protocol](http-server-protocol.md).
10. `server.address`/`server.port`: The value of `server.address` is attacker-controlled and may have high cardinality. Exercise care when using this for sampling decisions. Extracted per the semconv priority: `Forwarded#host` (RFC 7239) > `X-Forwarded-Host` > `:authority` > `Host`. `server.address` includes the port and `server.port` is not recorded separately; this is a semconv violation.
11. `url.path`: Recorded verbatim. No scrubbing is performed as the instrumentation cannot identify sensitive path content.
12. `url.query`: Not currently captured due to potentially sensitive information being included in the query string. APIs should manually log query parameters after sanitization. This is a semconv violation.

## Exception events

When a response body error occurs, the layer emits an event per the [semantic conventions for HTTP exception][otel-http-exceptions].

The layer does not emit events for request body errors. Instead, we rely on axum or the handlers to process these errors. This avoids recording a request body error twice.

Errors that occur during request handling (e.g. handler panics or axum converting a `Result` into a response) are not seen by this layer and do not produce exception events. Use `tower_http`'s `CatchPanicLayer` to emit exception events for panics.

The event is always named `http.server.request.exception`, is emitted at the ERROR level, and includes the following attributes:

* `exception.type`: The Rust type name of the error.
* `exception.message`: The `Display` representation of the error.
* `exception.stacktrace`: This representation isn't stable. It's intended to give a rough idea of the error context. Currently, the error chain is printed using `Display` representation of the error and its sources. We may use [`std::error::Report`](https://doc.rust-lang.org/std/error/struct.Report.html) or similar in the future.

Note: If the backend doesn't collect events for traces/spans, this information can be lost. Also, because the trace layer uses `tracing`, these events may show up in logs depending on how that's configured.
