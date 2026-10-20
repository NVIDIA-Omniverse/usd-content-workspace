(thumbnail-api-associations-and-resources-resources-associations-and-specifications)=
# Resources, associations, and specifications

The Thumbnail API is built from three shapes that appear on every transport and in every response: the
**Resource** that names a storage object, the **association** that links a thumbnail to the resource it
previews, and the **specification** that describes how the thumbnail was — or should be — produced. This page
defines each and the rules the service enforces on them.

Clients discover and manage thumbnails exclusively through the Thumbnail API. How an implementation persists
associations — in storage metadata, a database, or anything else — is not part of this specification, and
clients MUST NOT depend on any particular representation. Everything this page defines is observable through
the read, associate, and disassociate operations.

(thumbnail-api-associations-and-resources-resource)=
## Resource

A `Resource` identifies one storage object:

| Field               | Type   | Rule                                                                                                   |
|---------------------|--------|--------------------------------------------------------------------------------------------------------|
| `resource_address`  | string | Required, non-blank, at most 2048 characters. The storage URI of the object.                          |
| `resource_identity` | string | Optional. The opaque identity the storage backend assigns to a **file**. Required for files, MUST be omitted for folders. |

Associations are looked up by address. An implementation MUST treat an address written with raw characters and
the same address percent-encoded (spaces, non-ASCII text, `[ ] ^ | \` in the path) as the same address. The
identity is never canonicalized and never used for lookup.

A **thumbnail** is always a file and therefore always carries a `resource_identity`. The identity of a generated
thumbnail is the one the storage backend assigned when the thumbnail was written.

(thumbnail-api-associations-and-resources-association-targets)=
## Association targets

An association links a thumbnail to an **association target**, whose kind is given by `target_type`:

| `target_type` | Target                              | `association_target.resource_identity`      |
|---------------|-------------------------------------|---------------------------------------------|
| `FILE`        | A file — the default when omitted   | Required                                    |
| `FOLDER`      | A folder                            | MUST be omitted; a folder has no identity   |

Associations are looked up by the target's `resource_address` for both kinds. Reading, associating, and
disassociating a folder's thumbnails therefore needs only the folder address, and reading a file's thumbnails
needs only the file address — no identity is resolved on the read path.

(thumbnail-api-associations-and-resources-specifications)=
## Specifications

A specification's `type` selects a **processor** — the kind of conversion that produced, or will produce, the
thumbnail, such as image-to-image, video-to-image, video-to-video, USD-to-image, or USD-to-video. `MANUAL`
describes a thumbnail the caller produced. Each processor has its own parameters and its own encoding
enumeration (`ImageEncodingFormat`, `VideoEncodingFormat`, …). The type system enumerates the values expressible
on the wire; new processors and encodings are added there rather than by re-purposing existing values. The
Thumbnail API does not limit which processors, source formats, or encodings an implementation supports.

What the API requires of an implementation:

- An implementation MUST accept every specification kind the type system defines on the association API,
  regardless of which kinds it can itself generate. Accepting a specification for registration never commits
  an implementation to producing that kind — registering a thumbnail the caller already produced needs no
  conversion capability, whatever kind of thumbnail it is.
- An implementation MAY restrict the generation API to the kinds and encodings it can produce. If it does, it
  MUST reject a request naming an unsupported kind or encoding with `400 Bad Request` / `INVALID_ARGUMENT`
  naming the field, before any work is accepted.
- An implementation MUST NOT silently substitute a different kind or encoding for one it does not support.

The specification kinds the type system defines:

| Kind             | Fields                                                                                                                   | Meaning                                                                        |
|------------------|--------------------------------------------------------------------------------------------------------------------------|--------------------------------------------------------------------------------|
| `IMAGE_TO_IMAGE` | `width`, `height`, `encoding_format`, `transform_type`, `crop_strategy`, optional `quality`, optional `preserve_aspect_ratio` | The thumbnail was, or will be, an image — regardless of which processor produced it |
| `MANUAL`         | `width`, `height`, `encoding_format`                                                                                     | The caller produced the thumbnail bytes and is registering them; no conversion parameters exist |

(thumbnail-api-associations-and-resources-wire-shapes)=
### Wire shapes

The two operations that carry a specification use different JSON shapes over REST, for compatibility with the
clients that existed when each was introduced:

- `POST /v1alpha/associations` takes a **flat, `type`-tagged** object:

  ```json
  {
    "type": "IMAGE_TO_IMAGE",
    "width": 256,
    "height": 256,
    "encoding_format": "PNG",
    "transform_type": "FIT",
    "crop_strategy": "CENTER",
    "quality": 85,
    "preserve_aspect_ratio": true
  }
  ```

  A `MANUAL` specification is `{"type": "MANUAL", "width": 256, "height": 256, "encoding_format": "PNG"}`.
  Fields that do not belong to the selected kind are rejected.

- `POST /v1alpha/thumbnails` takes an **outer-keyed** object, `{"image_to_image": { ... }}`, whose outer key
  names the processor and whose inner object carries that processor's fields without a `type`. A `manual` key is
  not accepted there, because `MANUAL` describes a thumbnail the caller produced and nothing is generated for it.

Over gRPC both operations use the `ThumbnailSpecification` `oneof` with its `image_to_image` and `manual` arms.
Every enum is serialized as an UPPER_SNAKE_CASE string over REST and as the corresponding proto enum over gRPC.

(thumbnail-api-associations-and-resources-enumerations)=
### Enumerations

| Enumeration       | Values                                                                                                                                       |
|-------------------|----------------------------------------------------------------------------------------------------------------------------------------------|
| `encoding_format` | `PNG`, `JPEG`, `GIF`, `WEBP`                                                                                                                 |
| `transform_type`  | `SQUARE`, `SCALE_BY_WIDTH`, `SCALE_BY_HEIGHT`, `CROP`, `FIT`, `SCALE_AND_CROP`, `THUMBNAIL`, `RESIZE`                                       |
| `crop_strategy`   | `CENTER`, `TOP`, `BOTTOM`, `LEFT`, `RIGHT`, `TOP_LEFT`, `TOP_RIGHT`, `BOTTOM_LEFT`, `BOTTOM_RIGHT`, `SMART`                                  |
| `target_type`     | `FILE`, `FOLDER`                                                                                                                             |

Over gRPC, an enum field left at its `UNSPECIFIED` value where a value is required is rejected as
`INVALID_ARGUMENT` naming the field. `THUMBNAIL`, `RESIZE`, and `FIT` are distinct transforms and are never
remapped to one another.

(thumbnail-api-associations-and-resources-validation-rules)=
### Validation rules

| Rule                                                                                  | Applies to                             |
|---------------------------------------------------------------------------------------|----------------------------------------|
| `width` and `height` in `1..=8192` inclusive                                          | every specification, and the read filters |
| `quality` in `1..=100` inclusive when `encoding_format` is `JPEG` or `WEBP`           | `IMAGE_TO_IMAGE`; `PNG` and `GIF` are not range-checked |
| A specification is required                                                            | the association API                    |
| Omitted `quality` and `preserve_aspect_ratio` resolve to the implementation's configured defaults, once, before anything is recorded | `IMAGE_TO_IMAGE` |

Because omitted optional fields are resolved before anything is recorded, HTTP and gRPC requests that describe
the same thumbnail produce identical associations.

(thumbnail-api-associations-and-resources-stored-association-metadata)=
## What an association records

An association is the record that one thumbnail previews one association target. Whatever its representation,
an implementation MUST be able to return, for each association, the fields below — they are exactly what the
read API reports (see {ref}`REST API <thumbnail-api-rest-api-rest-api>` and {ref}`gRPC specification <thumbnail-api-grpc-grpc-specification>`):

| Field                | Content                                                                                                |
|----------------------|--------------------------------------------------------------------------------------------------------|
| `thumbnail`          | `resource_address` and `resource_identity` of the thumbnail file                                       |
| `association_target` | `resource_address` of the target; `resource_identity` for a `FILE` target, omitted for a `FOLDER`      |
| `target_type`        | `FILE` or `FOLDER`                                                                                     |
| `specification`      | The fully resolved specification of the selected processor; for `MANUAL`, `type`, `width`, `height`, and `encoding_format` |
| `create_time`        | When the association was recorded, RFC 3339                                                           |

Three guarantees hold regardless of representation:

- **One association per (target, thumbnail, specification).** Associating the same thumbnail with the same
  target and the same resolved specification again MUST replace the existing association rather than add a
  second one; the read API reports one entry.
- **Failure is not success.** If an association cannot be fully recorded, the implementation MUST NOT let it
  read back as an existing association.
- **Address equivalence.** An address written with raw characters and the same address percent-encoded refer to
  the same associations; `resource_identity` is never part of that equivalence.

(thumbnail-api-associations-and-resources-reading-associations)=
## Reading associations

`GET /v1alpha/thumbnails` and `GetThumbnails` return the associations recorded on one target address. Each
optional filter — `specification` (kind), `width`, `height`, `encoding_format`, `transform_type`,
`crop_strategy` — narrows the result by exact match when supplied and does not narrow it when omitted.

- A target with no associations, or none that match, is a successful read of nothing: `200 OK` / `OK` with an
  empty list, never a not-found error.
- Results are ordered by the thumbnail's `resource_address` and **capped at 1000 entries**. There is no
  continuation token; entries past the cap are discarded, so a response of exactly 1000 entries may be
  incomplete. Callers that need the remainder MUST narrow the query with filters.
- Numbers read back as integers over REST (`"width": 256`); over gRPC the specification travels as a
  `google.protobuf.Struct`, whose only numeric type is a double, so `256` arrives as `256.0`.

(thumbnail-api-associations-and-resources-idempotency)=
## Idempotency

| Operation                                            | Repeating it                                                                                         |
|------------------------------------------------------|------------------------------------------------------------------------------------------------------|
| Associate the same thumbnail with the same target and specification | Replaces the existing association; the read API reports one association               |
| Disassociate a thumbnail that is not associated       | Succeeds with empty `succeeded` and `failed` arrays — never a not-found error                        |
| Generate for the same source with the same specification | Produces a thumbnail at the same destination and replaces the same association                  |

Disassociation removes only the association. The thumbnail resource remains for its owner to manage; the
Thumbnail API has no operation that deletes a thumbnail's bytes.

(thumbnail-api-associations-and-resources-see-next)=
## See next

* {ref}`REST API <thumbnail-api-rest-api-rest-api>`
* {ref}`gRPC specification <thumbnail-api-grpc-grpc-specification>`
