# gRPC server metrics (A66)

See gRPC proposal [A66: OpenTelemetry Metrics](a66-otel-stats). Tracking [the finalized spec](../spec/A66-otel-stats.md).

A66 defines metrics that apply to streaming RPCs, but they provide limited insight for long-lived or high-message-count streams. The layer implements some custom metrics focused on streaming RPCs.

[a66-otel-stats]: https://github.com/grpc/proposal/blob/master/A66-otel-stats.md

## Contents

- [Available attributes](#available-attributes)
- [Metrics](#metrics)
  - [`grpc.server.call.started`](#grpcservercallstarted)
  - [`grpc.server.call.duration`](#grpcservercallduration)
  - [`grpc.server.call.sent_total_compressed_message_size`](#grpcservercallsent_total_compressed_message_size)
  - [`grpc.server.call.rcvd_total_compressed_message_size`](#grpcservercallrcvd_total_compressed_message_size)
  - [`grpc.server.handler.active`](#grpcserverhandleractive)
  - [`grpc.server.request.active`](#grpcserverrequestactive)
  - [`grpc.server.response.active`](#grpcserverresponseactive)

## Available attributes

`grpc.method`, `grpc.status`

Notes:
* The value of `grpc.method` is controlled by the client via the request URI. When no `RpcMap` is configured, the raw URI path is used, as there's no alternative. When an `RpcMap` is configured, methods not found in the map are recorded as `"other"` to prevent attackers triggering high cardinality, as per A66. You are strongly encouraged to build the layer with an RPC map.
* The `grpc.target` attribute does not apply to server metrics (per A66).
* The `grpc.status` value is not recorded for `grpc.server.call.rcvd_total_compressed_message_size`, see below.
* When no status code is found in the response, `grpc.status` is recorded as `UNKNOWN`.

## Metrics

Standard metrics:

* `grpc.server.call.started`: Counter (`{call}`)
* `grpc.server.call.sent_total_compressed_message_size`: Histogram (`By`)
* `grpc.server.call.rcvd_total_compressed_message_size`: Histogram (`By`)
* `grpc.server.call.duration`: Histogram (`s`)

Custom metrics:

* `grpc.server.handler.active`: UpDownCounter (`{handler}`)
* `grpc.server.request.active`: UpDownCounter (`{request}`)
* `grpc.server.response.active`: UpDownCounter (`{response}`)

### `grpc.server.call.started`

Incremented as soon as the layer receives a request stream.

Attributes:
* `grpc.method`

### `grpc.server.call.duration`

Timing starts as soon as the layer receives a request stream, and ends when the response stream completes. Care must be taken interpreting this metric when using long-running response streams.

Attributes:
* `grpc.method`
* `grpc.status`

### `grpc.server.call.sent_total_compressed_message_size`

>  Total bytes (compressed but not encrypted) sent across all response messages (metadata excluded) per RPC; does not include grpc or transport framing bytes.

Incomplete message bytes are included.

If an invalid frame is encountered, the raw (framed) bytes will be recorded as a fallback starting with the invalid frame.

Attributes:
* `grpc.method`
* `grpc.status`

### `grpc.server.call.rcvd_total_compressed_message_size`

> Total bytes (compressed but not encrypted) received across all request messages (metadata excluded) per RPC; does not include grpc or transport framing bytes.

Incomplete message bytes are included. Does not include unconsumed stream data: The layer doesn't buffer messages and Rust futures must be polled.

If an invalid frame is encountered, the raw (framed) bytes will be recorded as a fallback starting with the invalid frame.

For this metric, `grpc.status` is *not* recorded. See [recording response outcomes on request metrics](metrics-response-outcome-on-request.md). We'll consider recording this attribute if we have specific cases where it would have been useful and a design on how to handle the edge cases.

Attributes:
* `grpc.method`

### `grpc.server.handler.active`

The number of active handler futures, i.e. the Rust RPC implementation in a service (aka. "method").

Attributes:
* `grpc.method`

### `grpc.server.request.active`

The number of active request streams. This is independent of active handlers, e.g. for client or bi-directional streaming RPCs if the request stream processing is spawned off in the handler.

Attributes:
* `grpc.method`

### `grpc.server.response.active`

The number of active response streams. This is independent of active handlers, e.g. for server or bi-directional streaming RPCs if the handler returns a response stream.

Attributes:
* `grpc.method`
