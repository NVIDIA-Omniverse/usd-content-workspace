# Background

## How does distributed tracing work?

### Trace context creation

The `tracing` crate has a concept of span hierarchy, but not a trace context. This comes from Otel. Simplified, the trace context of a span is set by a "tracer" when the span is created. The default tracer attaches invalid i.e. empty trace contexts.

**Therefore, if you do not have a tracer in your client application, only invalid trace contexts are generated, and invalid trace contexts are not propagated.**

Luckily, when setting up a `opentelemetry_otlp`, it's [easy to specify a tracer](https://docs.rs/opentelemetry-otlp/0.26.0/opentelemetry_otlp/#kitchen-sink-full-configuration).

### Trace context propagation

The trace layers implement W3C trace context propagation using the `traceparent` and `tracestate` headers as described in the specification. The client trace layer ensures the request's span context is injected into requests, and the server trace layer ensures the request's (parent) span context is extracted from requests.

When using `opentelemetry`, the default is a global propagator via [`opentelemetry::global::set_text_map_propagator`](https://docs.rs/opentelemetry/0.26.0/opentelemetry/global/fn.set_text_map_propagator.html). The trace layers do not use the global propagator. It is not necessary to call `set_text_map_propagator()`, and the global text map propagator is ignored. Only W3C trace context propagation is supported.

### OpenTelemetry information/outcome propagation

The client and server trace layer ensures that the outcome of a request is recorded in the request span as per Otel RPC/gRPC semantic conventions. For a client, the outcome is either a response or a transport error. For a server, the outcome is either a response or a service error. The outcomes are also recorded slightly differently, which is why clients and servers have to use different layers.

## Why is this so hard?

### Many technologies involved

* [gRPC](https://grpc.io/about/) is a Remote Procedure Call (RPC) framework.
* gRPC uses [HTTP/2](https://httpwg.org/specs/rfc7540.html) for transport.
* [`tower`](https://crates.io/crates/tower) "is a library of modular and reusable components for building robust networking clients and servers."
* [`tonic`](https://crates.io/crates/tonic) is the de-facto standard Rust gRPC framework. It does not provide observability by default, but allows injecting `tower` layers into clients and servers.
* [`tokio`](https://tokio.rs/) is the most-widely used asynchronous framework for Rust.
* [`tracing`](https://crates.io/crates/tracing) "is a framework for instrumenting Rust programs". The `tracing` crate is used in e.g. Cargo, and is the default library for logging in asynchronous or threaded Rust code. The [`log`](https://crates.io/crates/log) crate is sometimes used for single-threaded Rust programs, but is generally unsuitable for use with `tonic`. Note that `tracing` was developed independently to OpenTelemetry by tokio.
* [`tracing-subscriber`](https://crates.io/crates/tracing-subscriber) allows to actually record `tracing` events. (This is because `tracing` is also intended for libraries.)
* [OpenTelemetry](https://opentelemetry.io/) aka. Otel is at it's core a specification for telemetry/observability, although it can also refer to the larger OpenTelemetry ecosystem.
* [`opentelemetry`](https://crates.io/crates/opentelemetry) is the "OpenTelemetry Rust API". On it's own, it doesn't do a whole lot, so other dependencies such as [`opentelemetry-sdk`](https://crates.io/crates/opentelemetry-sdk), [`opentelemetry-otlp`](https://crates.io/crates/opentelemetry-otlp), and [`opentelemetry-semantic-conventions`](https://crates.io/crates/opentelemetry-semantic-conventions) are usually needed for applications.
* [`tracing-opentelemetry`](https://crates.io/crates/tracing-opentelemetry) is essentially a bridge between `tracing` and `opentelemetry`.
* [W3C Trace Context](https://www.w3.org/TR/trace-context/) is a specification on how "to propagate context information that enables distributed tracing scenarios". (There exists others, e.g. AWS does not use trace context.) In short, this is required to be able to connect tracing information between e.g. a client and a server.

### Tracing the request/response flow is hard

HTTP/1.1 is a complex set of specifications. But the HTTP/1.1 request/response flow is fairly straightforward. The HTTP/2 flow can be complicated, and gRPC is another layer on-top of that.

The main culprit is streaming. Consider client streaming: The server may send a response code before the client has sent the request body (e.g. client headers, server headers, client body). The [gRPC docs on streaming](https://grpc.io/docs/what-is-grpc/core-concepts/) oversimplify this.

Additionally, `tonic` does not provide hooks to instrument the request/response flow, other than [`Server::trace_fn()`](https://docs.rs/tonic/latest/tonic/transport/struct.Server.html#method.trace_fn). Unfortunately, that does not allow recording the response status.

The other alternative is a `tower` layer/middleware. That means re-interpreting a HTTP/2 flow as gRPC, which is hard.
