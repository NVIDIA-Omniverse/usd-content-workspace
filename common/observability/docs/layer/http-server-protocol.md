# HTTP server protocol

The layers do not record `url.scheme` or `network.protocol.name`. This is a deliberate choice and an OTel semconv violation:

* `url.scheme` is required, even for HTTP.
* `network.protocol.name` may be omitted for HTTP. For the reasons below, this may be misleading.

Recording the attributes has two downsides. It increases telemetry volume, which is the main concern. Also, we cannot accurately determine whether the request was HTTP or HTTPS in the layer alone.

Although it is possible to serve HTTPS via axum, this is uncommon. More commonly, TLS termination is set up before axum. In this case, the server sees only HTTP. In some setups, it's possible to extract information from headers the reverse proxy sets up. Generally, it isn't safe to rely on such headers as they could be spoofed.

The semconv recommends falling back to the immediate peer scheme, which is HTTP. As mentioned, not only does this increase telemetry volume with little value. It also may cause issue analysis to draw incorrect conclusions based on only the axum server and not the complete server architecture. Because of this, we deliberately omit these attributes.
