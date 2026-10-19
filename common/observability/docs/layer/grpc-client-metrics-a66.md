# gRPC client metrics (A66)

See gRPC proposal [A66: OpenTelemetry Metrics](a66-otel-stats). Tracking [the finalized spec](../spec/A66-otel-stats.md).

A66 defines metrics that apply to streaming RPCs, but they provide limited insight for long-lived or high-message-count streams. The layer implements some custom metrics focused on streaming RPCs.

**Warning**: This layer doesn't handle retries. The metric layer must be applied so that it is called after a retry layer. Please see [the documentation on the order of layers for `tower::ServiceBuilder`](https://docs.rs/tower/latest/tower/struct.ServiceBuilder.html#order). Consequently, this layer doesn't record `grpc.client.call.duration`. We are looking into developing a retry layer that also records call/retry metrics.

[a66-otel-stats]: https://github.com/grpc/proposal/blob/master/A66-otel-stats.md

## Contents

- [Available attributes](#available-attributes)
- [Metrics](#metrics)
  - [`grpc.client.attempt.started`](#grpcclientattemptstarted)
  - [`grpc.client.attempt.duration`](#grpcclientattemptduration)
  - [`grpc.client.attempt.sent_total_compressed_message_size`](#grpcclientattemptsent_total_compressed_message_size)
  - [`grpc.client.attempt.rcvd_total_compressed_message_size`](#grpcclientattemptrcvd_total_compressed_message_size)
  - [`grpc.client.attempt.active`](#grpcclientattemptactive)
  - [`grpc.client.request.active`](#grpcclientrequestactive)
  - [`grpc.client.response.active`](#grpcclientresponseactive)

## Available attributes

`grpc.method`, `grpc.status`, `grpc.target`

Notes:
* The value of `grpc.method` is controlled by the client. The layer doesn't limit the cardinality of this value. Tonic clients constructed normally/from protobuf definitions inherently limit the cardinality.
* See the general layer documentation [on how to specify `grpc.target`](./grpc-layers.md). This is important, otherwise the value will be a default value (`other` per A109).
* The `grpc.status` value is not recorded for `grpc.client.attempt.sent_total_compressed_message_size`, see below.
* When no status code is found in the response, `grpc.status` is recorded as `UNKNOWN`.

## Metrics

Standard metrics:

* `grpc.client.attempt.started`: Counter (`{attempt}`)
* `grpc.client.attempt.duration`: Histogram (`s`)
* `grpc.client.attempt.sent_total_compressed_message_size`: Histogram (`By`)
* `grpc.client.attempt.rcvd_total_compressed_message_size`: Histogram (`By`)

Custom metrics:

* `grpc.client.attempt.active`: UpDownCounter (`{attempt}`)
* `grpc.client.request.active`: UpDownCounter (`{request}`)
* `grpc.client.response.active`: UpDownCounter (`{response}`)

### `grpc.client.attempt.started`

Incremented as soon as the layer receives a request.

Attributes:
* `grpc.method`
* `grpc.target`

### `grpc.client.attempt.duration`

Timing starts as soon as the layer receives a request, and ends when the response stream completes or a transport error occurs. Care must be taken interpreting this metric when using long-running streams.

Attributes:
* `grpc.method`
* `grpc.target`
* `grpc.status`

### `grpc.client.attempt.sent_total_compressed_message_size`

> Total bytes (compressed but not encrypted) sent across all request messages (metadata excluded) per RPC attempt; does not include grpc or transport framing bytes.

Incomplete message bytes are included.

If an invalid frame is encountered, the raw (framed) bytes will be recorded as a fallback starting with the invalid frame.

For this metric, `grpc.status` is *not* recorded. See [recording response outcomes on request metrics](metrics-response-outcome-on-request.md). We'll consider recording this attribute if we have specific cases where it would have been useful and a design on how to handle the edge cases.

Attributes:
* `grpc.method`
* `grpc.target`

### `grpc.client.attempt.rcvd_total_compressed_message_size`

> Total bytes (compressed but not encrypted) received across all response messages (metadata excluded) per RPC attempt; does not include grpc or transport framing bytes.

Incomplete message bytes are included. Does not include unconsumed stream data: The layer doesn't buffer messages and Rust futures must be polled.

If an invalid frame is encountered, the raw (framed) bytes will be recorded as a fallback starting with the invalid frame.

Not recorded if no response is received (e.g. due to a transport error). This means the data point count of the received metric may differ from the sent metric. As the request body is always created, the sent metric is always recorded.

Attributes:
* `grpc.method`
* `grpc.target`
* `grpc.status`

### `grpc.client.attempt.active`

The number of active client attempt futures, mirroring `grpc.server.handler.active` on the server side.

Attributes:
* `grpc.method`
* `grpc.target`

### `grpc.client.request.active`

The number of active request streams. This is independent of active call attempts.

Attributes:
* `grpc.method`
* `grpc.target`

### `grpc.client.response.active`

The number of active response streams. This is independent of active call attempts. Not recorded if no response is received (e.g. due to a transport error).

Attributes:
* `grpc.method`
* `grpc.target`
