# gRPC layers

Shared documentation for the gRPC client and server layers.

## Contents

- [`tracing` span name and location](#tracing-span-name-and-location)
- [Client layer target/address values](#client-layer-targetaddress-values)

## `tracing` span name and location

By default, the `tracing` span is created in the `tonic-observability` crate. This means:

* The trace layer span target name is hardcoded to either `client` or `server`. The span target is not used by OpenTelemetry. The exported span name is always set to the RPC path. The span target is used by `tracing` to filter traces, for example via `RUST_LOG`.
* The code file path and module name are visible in exported spans, and will point to the `tonic-observability` crate.

To customize the span creation, use the `.with_span()` method on the layer builder along with the `make_span!()` macro.

## Client layer target/address values

For `grpc.target` or `server.address`, the most reliable way to set this value is to specify a custom function via `with_target_fn` when the layer is built. The function can return any value, although it should be a valid URI. You should not include the request's path. That information is usually captured via `grpc.method` and would result in a high attribute value cardinality. The term "target" is a term of art specific to gRPC, see [gRPC name resolution](https://github.com/grpc/grpc/blob/master/doc/naming.md). TL;DR: return something like `http://server.internal.cluster/`, or whatever URI you pass when constructing the tonic endpoint/channel.

If no custom function is specified, a fallback function tries to derive the target from the request's URI. However, *this only works for clients constructed with an origin*. For clients constructed via `Client::with_origin()`, the origin's scheme and authority are used in the format `[{scheme}://]{authority}` (with no trailing slash). For clients constructed via `Client::new()`, the request's URI will always be relative, and so no meaningful value can be derived. This is a tonic limitation. In this case, the sentinel value `"other"` is used (per [gRPC A109 proposal](https://github.com/grpc/proposal/pull/528)). This avoids a split attribute/label set so that metric aggregations won't break.

For layers following OTel, `server.port` isn't recorded, only `server.address`. The OTel spec is a bit of a mess, we're treating this attribute as if it were `grpc.target` from the A66 gRPC metrics spec for consistency.
