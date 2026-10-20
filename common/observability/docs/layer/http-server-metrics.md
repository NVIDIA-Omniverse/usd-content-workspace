# HTTP server metrics

See [semantic conventions for HTTP metrics](otel-http-metrics). Tracking [v1.40.0](../spec/otel-http-metrics.md).

[otel-http-metrics]: https://opentelemetry.io/docs/specs/semconv/http/http-metrics/

Although the layer supports streaming, the OTel HTTP metrics are optimized for non-streaming, HTTP/1.1 servers.

## Contents

- [Available attributes](#available-attributes)
- [Metrics](#metrics)
  - [`http.server.request.started`](#httpserverrequeststarted)
  - [`http.server.request.duration`](#httpserverrequestduration)
  - [`http.server.request.body.size`](#httpserverrequestbodysize)
  - [`http.server.response.body.size`](#httpserverresponsebodysize)
  - [`http.server.active_requests`](#httpserveractive_requests)

## Available attributes

| OTel Attribute                            |    | Notes |
| ---                                       | -- | - |
| `error.type` (stable)                     | ✅ | 1 |
| `http.request.method` (stable)            | ⚠️ | 2 |
| `http.response.status_code` (stable)      | ✅ |   |
| `http.route` (stable)                     | ✅ | 3 |
| `network.protocol.name` (stable)          | ❌ | 4 |
| `network.protocol.version` (stable)       | ✅ |   |
| `server.address` (stable)                 | ❌ | 5 |
| `server.port` (stable)                    | ❌ | 5 |
| `url.scheme` (stable)                     | ❌ | 4 |
| `user_agent.synthetic.type` (development) | ❌ |   |

Notes:

1. `error.type`: If a response body error occurs, set to the error type name. Otherwise, if the response status code is a server error (5xx), set to the numerical status code per the example in the OTel semconv. For other status codes, this attribute is unset.
2. `http.request.method`: Canonicalized/case-insensitive due to `http`/`axum` limitations; `http` does not provide the non-canonicalized value. `OTEL_INSTRUMENTATION_HTTP_KNOWN_METHODS` is not implemented; this is a semconv violation.
3. `http.route`: Only set if `axum::extract::MatchedPath` is available.
4. `network.protocol.name`, `url.scheme`: These attributes aren't recorded. This is a deliberate choice and an OTel semconv violation, see [HTTP server protocol](http-server-protocol.md).
5. `server.address`, `server.port`: Omitted as these values are derived entirely from client-supplied headers (`Forwarded`, `X-Forwarded-Host`, `:authority`, `Host`) which allow attackers to trigger high cardinality.

## Metrics

| Metric                                         |    | Instrument |
| ---                                            | -- | --- |
| `http.server.request.started` (custom)         | ✅ | Counter (`{request}`) |
| `http.server.request.duration` (stable)        | ✅ | Histogram (`s`) |
| `http.server.request.body.size` (development)  | ✅ | Histogram (`By`) |
| `http.server.response.body.size` (development) | ✅ | Histogram (`By`) |
| `http.server.active_requests` (development)    | ❌ | UpDownCounter (`{request}`) |

### `http.server.request.started`

Custom metric, not part of the OTel spec. Incremented as soon as the layer receives a request. Mirrors `grpc.server.call.started`.

Attributes:

* `http.request.method`
* `network.protocol.version`
* `http.route`

### `http.server.request.duration`

Measured from when the service is called, to when the response body is dropped, the same as the span duration of the trace layer.

Attributes:

* `http.request.method`
* `network.protocol.version`
* `http.route`
* `http.response.status_code`
* `error.type`

### `http.server.request.body.size`

Total size of the consumed request body (in bytes). The consumed size is recorded due to how Rust futures work; the layer does not buffer any data.

Doesn't include headers or trailers. The size will be zero for e.g. a GET request, a body that only contains trailers, or a body that the handler doesn't consume.

For this metric, `error.type` and `http.response.status_code` are *not* recorded. This is a semconv violation. See [recording response outcomes on request metrics](metrics-response-outcome-on-request.md). We'll consider recording these attributes if we have specific cases where they would have been useful and a design on how to handle the edge cases.

Attributes:

* `http.request.method`
* `network.protocol.version`
* `http.route`

### `http.server.response.body.size`

Total size of the consumed response body (in bytes).

Doesn't include headers or trailers. The size will be zero if a handler doesn't return a body (for example, only returning a status code) or if the body only contains trailers.

Attributes:

* `http.request.method`
* `network.protocol.version`
* `http.route`
* `http.response.status_code`
* `error.type`

Note: Per the semconv, `http.response.status_code` is the status code that was sent and `error.type` is the error the operation ended with. Therefore, it is possible to have metrics with e.g. 200 status codes, but an error set when a response body error occurs.

### `http.server.active_requests`

Not implemented. This metric is under-specified: even without streaming, it's unclear if it is decremented as soon as the response is sent, or after the response body completes. In future, we'd prefer to implement metrics that track the number of active request streams, handlers, and response streams, similar to the custom A66-based gRPC metrics.
