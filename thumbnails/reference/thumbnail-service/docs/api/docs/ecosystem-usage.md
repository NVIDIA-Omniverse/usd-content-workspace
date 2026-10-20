(thumbnail-api-ecosystem-usage-ecosystem-usage)=
# Ecosystem usage

This page describes how the Thumbnail API fits into the wider Omniverse storage ecosystem: the problem it
solves, who calls it, and the workflows it participates in. It does not describe the request/response shapes —
see {ref}`Thumbnail generation <thumbnail-api-thumbnail-generation-thumbnail-generation>`,
{ref}`REST API <thumbnail-api-rest-api-rest-api>`, and {ref}`gRPC specification <thumbnail-api-grpc-grpc-specification>`
for those.

(thumbnail-api-ecosystem-usage-why-the-api-exists)=
## Why the API exists

Users of a storage service want to see visual previews of the files they add, but rendering a preview on demand
in every client that displays one is wasteful and produces inconsistent results across clients. The Thumbnail
API centralizes that work: an implementation generates previews once and makes the resulting associations
queryable through a stable API, so every caller reads the same thumbnails instead of each rendering its own.

(thumbnail-api-ecosystem-usage-who-calls-it)=
## Who calls it

The API's callers are developers integrating with an Omniverse storage-service stack — applications and
services that need to display previews of files held in storage (for example, a file browser, an asset
manager, or a content pipeline). The API surface is the product these callers build against; end users
experience it indirectly, through whatever client renders the previews it returns.

(thumbnail-api-ecosystem-usage-workflows-it-participates-in)=
## Workflows it participates in

A caller integrates with the Thumbnail API through one or both of these workflows:

- **Event-driven generation.** Where an implementation supports it, thumbnails are generated automatically as
  qualifying files land in storage, with no explicit client call required. A caller that only needs previews to
  exist eventually integrates by reading the association API rather than triggering generation itself.
- **On-demand generation.** A caller names a source resource and, optionally, the specifications it wants,
  and the API produces the requested thumbnails — either before it answers or after, depending on the
  implementation (see {ref}`Thumbnail generation <thumbnail-api-thumbnail-generation-thumbnail-generation>`).
  A caller that needs a thumbnail for a specific source right away integrates by calling the generation API
  directly.

Both workflows converge on the same association model: whether a thumbnail was generated automatically or on
demand, or registered by a caller that produced it itself, it is discovered the same way — by reading the
associations recorded on its source.

(thumbnail-api-ecosystem-usage-what-callers-can-rely-on)=
## What callers can rely on

- **One authority for source-to-thumbnail links.** The API is the record of which thumbnails belong to which
  source; a caller never needs its own bookkeeping to find a thumbnail it has already seen generated or
  registered.
- **Consistent behavior across transports.** REST and gRPC callers integrate against the same operations,
  validation, and stored results — a workflow is not tied to one transport.
- **No assumption about completion timing.** A caller may receive thumbnails synchronously or discover them
  later through the association read API or change notifications; a workflow that assumes one or the other is
  not portable across implementations.

(thumbnail-api-ecosystem-usage-see-next)=
## See next

* {ref}`Thumbnail generation <thumbnail-api-thumbnail-generation-thumbnail-generation>`
* {ref}`Resources, associations, and specifications <thumbnail-api-associations-and-resources-resources-associations-and-specifications>`
* {ref}`REST API <thumbnail-api-rest-api-rest-api>`
* {ref}`gRPC specification <thumbnail-api-grpc-grpc-specification>`
