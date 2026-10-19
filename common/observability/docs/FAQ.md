# FAQ

## Tonic Observability

### Tracing layers

**Q**: Why are there separate server and client layers?

**A**: For servers e.g. an `InvalidArgument` response is not considered an error, and so isn't logged as such. For a client, that would be logged as an error.

**Q**: For servers, does the trace layer automatically instrument a gRPC service?

**A**: Yes. The layer sits in front of a the service, and even handles calls to RPC functions the server does not implement. While the layer does capture an entire streaming response, it is not able to "see" inside the stream. So it's still important to instrument the handlers in detail. Additionally, in the case of client streaming, the scenario of spawning the stream off and returning a response before the client stream is fully processed isn't currently tested.

**Q**: For clients, does the trace layer automatically instrument a client?

**A**: Yes, but response futures must be polled. The layer is not able to see what you do with the response, so it's still important to instrument client code in detail.

**Q**: Do I have to call `opentelemetry::global::set_text_map_propagator`?

**A**: No. The layers do not use the global text map propagator.

**Q**: I'm calling `opentelemetry::global::set_text_map_propagator`, but the layers are still using W3C Trace Context headers?

**A**: The layers do not use the global text map propagator. There is no option to use a different trace context propagation method.

**Q**: Is is possible to ignore incoming W3C Trace Context information?

**A**: There is a workaround for this: Add a new layer before the server trace layer that strips the corresponding headers. The only scenario I'm aware of where this is necessary is dealing with malicious clients. If you need this, please reach out if.

### Metrics layers

**Q**: Why are there separate server and client layers?

**A**: The metrics have different names. Additionally, errors are handled differently as with tracing.

**Q**: For servers, does the metric layer automatically instrument a gRPC service?

**A**: Yes. The layer sits in front of a the service, and even handles calls to RPC functions the server does not implement.
