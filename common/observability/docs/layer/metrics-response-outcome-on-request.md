# Recording response outcomes on request metrics

Both HTTP/OTel and gRPC/A66 metrics want to record the response outcome on certain request metrics. This is reasonable, but difficult in streaming scenarios.

Currently, response outcome attributes are omitted on request metrics to reduce layer complexity and because we aren't aware of a use-case. If you have a use-case, please reach out.

## Details

Before implementing the attributes - especially if using AI/LLMs - consider the following scenarios from the server perspective:

* The handler processes the request stream and then sends a response (unary, server streaming, client streaming where the request stream is consumed). Most obvious case.
* The handler spawns off the request stream processing to another thread/task, allowing the handler to potentially send a response while the request stream hasn't ended (client streaming with acknowledgement/background processing, for example for receiving events).
* Bi-directional streaming scenarios that may have completely arbitrary request/response patterns.

In the case where the response stream ends after the request stream, recording of the metric must be delayed until the response stream has ended and the request information has to be transferred from the request steam (which may be dropped) to the response stream (or another entity) to record the metric. Recording the metric may be delayed significantly as there is no time-limit on server streaming.

In the case where the request stream processing is spawned off, the request stream may end after the response stream. In this case, the response information has to be transferred to the request stream (or another entity) for recording the metric.

In other words, a request metric can only be recorded once both the request and response stream have ended. So why isn't this an issue for the trace layers? It is. However, the span is shared between the request and response stream. Currently, the attributes being recorded are orthogonal. So recording an attribute twice from the request and response streams doesn't occur (but it could!).
