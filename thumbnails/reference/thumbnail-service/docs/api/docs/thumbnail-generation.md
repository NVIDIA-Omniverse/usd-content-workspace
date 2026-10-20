(thumbnail-api-thumbnail-generation-thumbnail-generation)=
# Thumbnail generation

A thumbnail is produced from a **source resource** and one **specification**. This page describes what can
trigger generation, what a client can rely on once a request is accepted, how specifications are resolved, and
the limits every request is subject to. It does not prescribe how an implementation performs the conversion,
where it queues work, or how many components it uses.

This page refers to the operations by role rather than by transport. The **generation API** is
`POST /v1alpha/thumbnails` over REST and `CreateThumbnails` over gRPC; the **association API** is
`POST /v1alpha/associations` / `AssociateThumbnail`; the **association read API** is `GET /v1alpha/thumbnails` /
`GetThumbnails`. Transport-specific request and response shapes are in {ref}`REST API <thumbnail-api-rest-api-rest-api>` and
{ref}`gRPC specification <thumbnail-api-grpc-grpc-specification>`.

(thumbnail-api-thumbnail-generation-lifecycle-of-a-generation-request)=
## How generation is triggered

A thumbnail service can generate thumbnails in two ways; an implementation MAY support either or both:

- **Asynchronous flow.** Generation is triggered by events that reach the service from other systems and
  services — for example storage notifications, or events delivered through an event broker. No client request
  is involved: the implementation decides which events are eligible and which specifications to produce
  (typically a configured default set), and the party that caused the event is not notified directly.
- **Synchronous flow.** Generation is triggered on demand by a client request to the generation API
  (`POST /v1alpha/thumbnails` or `CreateThumbnails`) naming the source `resource_address` and, optionally, the
  specifications wanted. When the client supplies no specifications, the implementation applies its configured
  defaults, so both flows produce the same thumbnails for the same source.

Whichever flow triggers it, one generation request resolves into one thumbnail per specification, and each
thumbnail the implementation produces is recorded as an **association** between the source and the thumbnail,
carrying the fully resolved specification (see
{ref}`Resources, associations, and specifications <thumbnail-api-associations-and-resources-resources-associations-and-specifications>`). From then on the thumbnail
is visible through `GET /v1alpha/thumbnails` and `GetThumbnails`, and through any change notification the
implementation publishes.

(thumbnail-api-thumbnail-generation-acceptance-is-not-completion)=
## What the generation API returns

What a caller receives depends on which flow triggered the generation:

- **Asynchronous flow.** Nothing is returned to a client, because no client made a request. Whoever caused the
  triggering event learns of the result — if at all — through the association read API, or through a change
  notification where the implementation publishes one.
- **Synchronous flow.** The generation API answers the client. An implementation completes the request in one
  of two ways and signals which by the status; which one it uses is a property of the deployment, not of the
  request, and callers MUST treat either as success:
  - **Asynchronous completion** — `202 Accepted` over REST, `OK` over gRPC — as soon as at least one
    generation request has been accepted for processing. No thumbnail need exist yet, and the conversion can
    still fail afterwards, for example because the source is not a decodable image or exceeds the
    implementation's size bounds.
  - **Synchronous completion** — `200 OK` over REST, `OK` over gRPC — once the thumbnails have been generated,
    written, and associated. Every specification absent from `failed` exists and is readable through the
    association read API at that moment.

  Over gRPC both modes return `OK`; a caller tells them apart by whether the `succeeded` entries carry a
  `resource_identity`, which is present only once a thumbnail exists.

The response has the same shape in both modes. `failed` (the JSON body over REST, `CreateThumbnailsResponse.failed`
over gRPC) names every specification that was **not** handled, together with the reason — under asynchronous
completion, why it could not be accepted; under synchronous completion, why the thumbnail could not be produced.
An empty list means every specification was handled. Callers MUST NOT compare the length of `failed` with the
number of specifications they sent: when the request carries no specifications, the implementation fans out
over its configured defaults, so `failed` can describe specifications the caller never wrote and can be longer
than the caller's list.

`succeeded` names each handled specification with the thumbnail's address — and, once the thumbnail exists, its
identity — so a caller of a synchronous implementation does not need a second read to find its thumbnails.

A valid request in which **no** specification could be handled is not a partial result; it fails with the
status of its cause. Under asynchronous completion that is `502 Bad Gateway` / `INTERNAL`: the source could not
be looked up, or nothing could be handed to the system that performs generation. Under synchronous completion it
is `422 Unprocessable Content` / `FAILED_PRECONDITION` when the source cannot be processed, or
`503 Service Unavailable` / `UNAVAILABLE` for a transient dependency failure or load shedding. A request that is
itself invalid — an invalid specification, a source that does not exist, a missing or rejected token — is
reported with its own `4xx` / gRPC status and accepts nothing. The {ref}`REST API <thumbnail-api-rest-api-rest-api>` and
{ref}`gRPC specification <thumbnail-api-grpc-grpc-specification>` pages list every status.

To learn that a thumbnail exists, read the associations of the source through the association read API, or
consume a change notification where the implementation publishes one. The API defines no task identifier and no
status endpoint.

(thumbnail-api-thumbnail-generation-specifications-and-defaults)=
## Specifications and defaults

A **specification** describes one thumbnail. For an `IMAGE_TO_IMAGE` specification the fields are `width`,
`height`, `encoding_format`, `transform_type`, `crop_strategy`, and two optional fields, `quality` and
`preserve_aspect_ratio`. `MANUAL` specifications describe a thumbnail the caller produced and belong to the
association API; the generation API does not generate them, because no conversion takes place (see
{ref}`Resources, associations, and specifications <thumbnail-api-associations-and-resources-resources-associations-and-specifications>`).

A caller MAY omit any optional field. Omission is distinct from an explicit value:

| Field                   | When omitted                                                                                                 |
|-------------------------|--------------------------------------------------------------------------------------------------------------|
| `specifications`        | One thumbnail per configured size, using every configured processing default                                |
| `quality`               | The deployment's configured default quality is applied                                                       |
| `preserve_aspect_ratio` | The deployment's configured default is applied; an explicit `false` is honored as `false`                   |

The specification an implementation acts on is always complete. Configuration-derived and caller-supplied values
are indistinguishable on the wire, and the stored association records the resolved values, so reading an
association back returns the specification that was actually used.

Which source formats an implementation can decode, which encodings it produces, and how it realizes each
`transform_type` and `crop_strategy` are properties of the implementation, not of this specification. The
reference implementation documents its own in the Thumbnail Service documentation (*Generation pipeline*).

(thumbnail-api-thumbnail-generation-where-thumbnails-are-written)=
## Where thumbnails are written

The implementation chooses the destination address of every thumbnail; callers do not choose it and cannot
override it per request. The destination MUST be a deterministic function of the source `resource_address`, the
source `resource_identity`, and the resolved specification, so that:

- the same source, identity, and specification always resolve to the same destination, and regenerating a
  thumbnail overwrites the previous one rather than accumulating copies;
- whether a new version of a source produces a new thumbnail address or overwrites the old one is the
  implementation's choice — whether it folds the source identity into the destination. The association read API
  always reports the address that was actually written.

An implementation writes to the destination it resolved and nothing else. A generated thumbnail is itself a
storage resource, so an event-driven implementation needs to recognize its own output and not generate
thumbnails of thumbnails.

(thumbnail-api-thumbnail-generation-limits)=
## Limits

| Limit                                   | Value                                          | Violation                                              |
|-----------------------------------------|------------------------------------------------|--------------------------------------------------------|
| `width`, `height`                       | `1..=8192` inclusive                           | `400 Bad Request` / `INVALID_ARGUMENT`, before the source is looked up |
| `quality` for `JPEG` and `WEBP`         | `1..=100` inclusive                            | `400 Bad Request` / `INVALID_ARGUMENT`, before the source is looked up |
| `quality` for `PNG` and `GIF`           | not range-checked                              | —                                                      |
| Specifications per generation request   | at most `1000`                                 | `400 Bad Request` / `INVALID_ARGUMENT`, before the source is looked up |
| `resource_address`                      | required, non-blank (the schema advertises a `2048`-character maximum) | `400 Bad Request` / `INVALID_ARGUMENT` when blank; a longer address is resolved against storage and fails with `404` / `NOT_FOUND` if it does not exist |

One invalid specification rejects the whole request: nothing is accepted until every specification passes, so
the caller can correct its input without leaving partial work behind. Source images are additionally bounded by
the implementation — typically by encoded size, decoded pixel count, and peak conversion memory. A source that
exceeds those bounds produces no thumbnail; under synchronous completion the failure is reported in `failed` or
as `422 Unprocessable Content` / `FAILED_PRECONDITION`, under asynchronous completion it is not visible through
the generation API and is discovered through the association read API.

(thumbnail-api-thumbnail-generation-see-next)=
## See next

* {ref}`Resources, associations, and specifications <thumbnail-api-associations-and-resources-resources-associations-and-specifications>`
* {ref}`REST API <thumbnail-api-rest-api-rest-api>`
* {ref}`gRPC specification <thumbnail-api-grpc-grpc-specification>`
