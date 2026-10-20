(thumbnail-api-rest-api-rest-api)=
# REST API

The Thumbnail API defines four REST endpoints:
- `POST /v1alpha/thumbnails` – triggers thumbnail generation for a source file;
- `GET /v1alpha/thumbnails` – reads the thumbnails associated with a target resource;
- `POST /v1alpha/associations` – records that an existing thumbnail belongs to a target resource;
- `DELETE /v1alpha/associations` – removes one thumbnail's association with a target resource.

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this section are to be interpreted as described in [RFC 2119](https://datatracker.ietf.org/doc/html/rfc2119).

Every `/v1alpha` endpoint requires an `Authorization: Bearer <accessToken>` header. An implementation MUST
reject an unauthenticated request before parsing the body, so a caller without a credential does not receive a
description of the expected schema. A missing, invalid, or expired token is reported as `401 Unauthorized`; a
valid token that is not authorized for the resource is reported as `403 Forbidden` where this specification
defines it. How the implementation reaches that decision — validating the token itself, or asking its storage
backend — is not part of the API.

Every error response carries the same body shape:

```text
HTTP/1.1 400 Bad Request
Content-Type: application/json

{
   "source": "thumbnail-service",
   "error_message": "<description of the problem>"
}
```

`source` is REQUIRED. `error_message` is REQUIRED for `400 Bad Request` and `404 Not Found` and SHOULD name the
offending field or resource so the caller can correct the request. For `401 Unauthorized` and `403 Forbidden`
an implementation MAY omit `error_message` or return a generic value such as `"Unauthorized"` or
`"Forbidden"`; if it returns more, the message SHOULD NOT disclose why the credential was rejected, which
component made the decision, or any text originating from a backend. An optional `details` field MAY carry
structured detail; `details` SHOULD NOT be present on `401` or `403`. Clients MUST NOT parse `error_message`;
its wording is illustrative throughout this document. Error messages are truncated to 4096 characters.

(thumbnail-api-rest-api-post-v1alpha-thumbnails)=
## POST /v1alpha/thumbnails

```text
POST /v1alpha/thumbnails HTTP/1.1
Authorization: Bearer <accessToken>
Content-Type: application/json

{
   "resource_address": <the source file's storage address>,
   "specifications": [
      {"image_to_image": <an image-to-image specification>},
      ...
   ]
}
```

This API triggers thumbnail generation for one source file. The implementation resolves the source's
`resource_identity` from its storage backend; callers supply only `resource_address`.

`resource_address` is REQUIRED and MUST be non-blank.

`specifications` is OPTIONAL. When it is absent or empty, the implementation generates one thumbnail per
configured default using its configured processing defaults — identical to the event-driven flow. When it is
present, every entry MUST carry a specification of a kind the implementation supports for generation; a
`MANUAL` specification, or an entry that names no kind, is rejected because nothing is generated for a
caller-produced thumbnail. At most `1000` specifications MAY be sent in one request.

An `image_to_image` object carries `width`, `height`, `encoding_format`, `transform_type`, `crop_strategy`, and
the OPTIONAL `quality` and `preserve_aspect_ratio` fields described in
{ref}`Resources, associations, and specifications <thumbnail-api-associations-and-resources-resources-associations-and-specifications>`. `width` and `height` MUST be in
`1..=8192`. `quality` MUST be in `1..=100` when `encoding_format` is `JPEG` or `WEBP`; for lossless encodings an
implementation MAY ignore `quality`. Omitted OPTIONAL fields resolve to the deployment's configured defaults.

All of these checks run before the source is looked up. One invalid specification rejects the whole request and
accepts nothing.

The request example:

```text
POST /v1alpha/thumbnails HTTP/1.1
Authorization: Bearer <access-token>
Content-Type: application/json

{
   "resource_address": "https://account.blob.core.windows.net/container/photos/image.jpg",
   "specifications": [
      {
         "image_to_image": {
            "width": 256,
            "height": 256,
            "encoding_format": "PNG",
            "transform_type": "FIT",
            "crop_strategy": "CENTER",
            "preserve_aspect_ratio": true
         }
      },
      {
         "image_to_image": {
            "width": 1024,
            "height": 768,
            "encoding_format": "JPEG",
            "transform_type": "SCALE_AND_CROP",
            "crop_strategy": "SMART",
            "quality": 90
         }
      }
   ]
}
```

The `specifications` field MAY be omitted to generate every configured size:

```text
POST /v1alpha/thumbnails HTTP/1.1
Authorization: Bearer <access-token>
Content-Type: application/json

{
   "resource_address": "https://account.blob.core.windows.net/container/photos/image.jpg"
}
```

An implementation completes this call in one of two ways and signals which by the status code. Callers MUST
treat either as success and MUST NOT assume one or the other; which mode an implementation uses is a property
of the deployment, not of the request.

- **`202 Accepted` — asynchronous completion.** Every generation request was accepted for processing later. No
  thumbnail need exist yet; the caller discovers them through `GET /v1alpha/thumbnails` (see
  {ref}`What the generation API returns <thumbnail-api-thumbnail-generation-acceptance-is-not-completion>`).
- **`200 OK` — synchronous completion.** The implementation generated, wrote, and associated the thumbnails
  before answering. Every specification absent from `failed` exists and is readable through
  `GET /v1alpha/thumbnails` at the moment the response arrives.

The body has the same shape in both modes: `succeeded` names each specification that was handled and where its
thumbnail is, `failed` names each that was not. Under `200 OK` every `succeeded` entry exists and carries
`resource_identity`; under `202 Accepted` entries carry the destination address and no identity, and an
implementation MAY return `succeeded` empty. Full success:

```text
HTTP/1.1 202 Accepted
Content-Type: application/json

{
   "succeeded": [
      {
         "specification": {
            "image_to_image": {
               "width": 256,
               "height": 256,
               "encoding_format": "PNG",
               "transform_type": "FIT",
               "crop_strategy": "CENTER",
               "quality": 85,
               "preserve_aspect_ratio": true
            }
         },
         "thumbnail": {
            "resource_address": "https://account.blob.core.windows.net/container/.thumbs/photos/image.jpg/256x256.png"
         }
      }
   ],
   "failed": []
}
```

Under synchronous completion the same body is returned with `HTTP/1.1 200 OK`, and each `succeeded[].thumbnail`
also carries `resource_identity`.

When some, but not all, specifications could be handled, the status is unchanged (`202` or `200`) and `failed`
names each specification that was not, in its fully resolved form, with the reason. Under asynchronous
completion the reason is why the request could not be accepted; under synchronous completion it is why the
thumbnail could not be produced:

```text
HTTP/1.1 202 Accepted
Content-Type: application/json

{
   "succeeded": [],
   "failed": [
      {
         "specification": {
            "image_to_image": {
               "width": 1024,
               "height": 768,
               "encoding_format": "JPEG",
               "transform_type": "SCALE_AND_CROP",
               "crop_strategy": "SMART",
               "quality": 90,
               "preserve_aspect_ratio": true
            }
         },
         "error_message": "publish deadline exceeded"
      }
   ]
}
```

```text
HTTP/1.1 200 OK
Content-Type: application/json

{
   "succeeded": [
      {
         "specification": {
            "image_to_image": {
               "width": 256,
               "height": 256,
               "encoding_format": "PNG",
               "transform_type": "FIT",
               "crop_strategy": "CENTER",
               "quality": 85,
               "preserve_aspect_ratio": true
            }
         },
         "thumbnail": {
            "resource_address": "https://account.blob.core.windows.net/container/.thumbs/photos/image.jpg/256x256.png",
            "resource_identity": "fileobject:thumb-abc123:v7"
         }
      }
   ],
   "failed": [
      {
         "specification": {
            "image_to_image": {
               "width": 1024,
               "height": 768,
               "encoding_format": "JPEG",
               "transform_type": "SCALE_AND_CROP",
               "crop_strategy": "SMART",
               "quality": 90,
               "preserve_aspect_ratio": true
            }
         },
         "error_message": "source could not be decoded as an image"
      }
   ]
}
```

Callers MUST NOT compare `failed.length` with `specifications.length`: with an absent or empty `specifications`
the implementation fans out over its configured defaults, so `failed` can describe specifications the caller
never sent. A `202` always means at least one specification was accepted, and a `200` always means at least one
thumbnail was produced; a request in which nothing was accepted or nothing was produced does not return a
success status with a fully populated `failed` list — it fails with the status of its cause, listed under
{ref}`Error conditions <thumbnail-api-rest-api-post-v1alpha-thumbnails-error-conditions>`.
`502 Bad Gateway` is reserved for one case: the request was valid, but under asynchronous completion the
implementation could not hand any of it to the downstream system that performs generation, or could not reach
its storage backend to look up the source. A problem with the request itself — a malformed body, an invalid
specification, a missing or rejected token, a source that does not exist — is never reported as `502`.

(thumbnail-api-rest-api-post-v1alpha-thumbnails-error-conditions)=
### Error conditions

Every status this endpoint can return, in one place:

| Status                       | Meaning                                                                                       |
|------------------------------|-----------------------------------------------------------------------------------------------|
| `200 OK`                     | Request valid; generation completed synchronously — a specification absent from `failed` has been produced and associated |
| `202 Accepted`               | Request valid; at least one generation request accepted for asynchronous completion (`failed` lists any that were not) |
| `400 Bad Request`            | Request invalid — refused before the storage backend is contacted                              |
| `401 Unauthorized`           | Missing, malformed, or rejected token                                                          |
| `403 Forbidden`              | Token valid, caller not authorized to read the source                                          |
| `404 Not Found`              | Source `resource_address` does not exist                                                       |
| `422 Unprocessable Content`  | Synchronous completion: the source cannot be processed (undecodable, unsupported, over bounds) |
| `502 Bad Gateway`            | Asynchronous completion: request valid, but nothing could be handed downstream or the source lookup could not complete |
| `503 Service Unavailable`    | Synchronous completion: transient dependency failure or load shedding; retry                    |

The following error conditions are defined:

**Missing, malformed, invalid, or expired credential**

```text
HTTP/1.1 401 Unauthorized
Content-Type: application/json

{
   "source": "thumbnail-service",
   "error_message": "Unauthorized"
}
```

**The credential is valid but the caller is not authorized for the source**

```text
HTTP/1.1 403 Forbidden
Content-Type: application/json

{
   "source": "thumbnail-service",
   "error_message": "Forbidden"
}
```

**The request is invalid — a blank `resource_address`, more than 1000 specifications, a specification without
`image_to_image`, a dimension outside `1..=8192`, or a JPEG/WebP quality outside `1..=100`.** These are refused
before the storage backend is contacted.

```text
HTTP/1.1 400 Bad Request
Content-Type: application/json

{
   "source": "thumbnail-service",
   "error_message": "specification 1: quality must be between 1 and 100 for JPEG/WebP"
}
```

Other messages of this class: `specification 0: width must be between 1 and 8192`,
`specifications must contain at most 1000 entries`,
`specifications[0]: only image_to_image specifications are supported for CreateThumbnails; the image_to_image field must be set`,
and `invalid JSON body: ...` for a body that does not deserialize.

**The source `resource_address` does not exist**

```text
HTTP/1.1 404 Not Found
Content-Type: application/json

{
   "source": "thumbnail-service",
   "error_message": "resource not found in storage"
}
```

**Synchronous completion: no specification produced a thumbnail because the source cannot be processed** — it is
not a decodable image, its format is not supported by the implementation, or it exceeds the implementation's
decode bounds. The message names the cause.

```text
HTTP/1.1 422 Unprocessable Content
Content-Type: application/json

{
   "source": "thumbnail-service",
   "error_message": "source could not be decoded as an image"
}
```

**Synchronous completion: a transient failure prevented every thumbnail** — the storage backend or the
association write failed transiently, or the implementation shed the request because its in-flight conversion
bound was reached. The request MAY succeed if retried.

```text
HTTP/1.1 503 Service Unavailable
Retry-After: 5
Content-Type: application/json

{
   "source": "thumbnail-service",
   "error_message": "conversion capacity exhausted; retry later"
}
```

**The request was valid, but a downstream dependency failed: the source lookup could not complete, or no
generation request could be handed to the system that performs generation.** A failed lookup accepts nothing; a
hand-off that timed out MAY nevertheless have been received downstream, so the caller SHOULD read the
associations before retrying.

```text
HTTP/1.1 502 Bad Gateway
Content-Type: application/json

{
   "source": "thumbnail-service",
   "error_message": "all thumbnail task submissions failed: ..."
}
```

`502` applies to asynchronous completion only; a synchronous implementation has no downstream system to hand
work to and reports its own failures as `422` or `503` above.

(thumbnail-api-rest-api-get-v1alpha-thumbnails)=
## GET /v1alpha/thumbnails

```text
GET /v1alpha/thumbnails?association_target_resource_address=<target address>[&specification=<kind>][&width=<n>][&height=<n>][&encoding_format=<format>][&transform_type=<type>][&crop_strategy=<strategy>] HTTP/1.1
Authorization: Bearer <accessToken>
```

This API returns the thumbnails associated with one target resource. It is read-only.

`association_target_resource_address` is REQUIRED, MUST be non-blank, and MUST NOT exceed 2048 characters.
Associations are keyed on the target's address for `FILE` and `FOLDER` targets alike, so no identity is
supplied or resolved.

Every other parameter is an OPTIONAL filter. A supplied filter MUST match a stored value exactly; an omitted
filter does not narrow the result. `specification` selects the specification kind to match, named as in a
specification's `type` (for example `IMAGE_TO_IMAGE`). `width` and `height` MUST be in `1..=8192`.
`transform_type` and `crop_strategy` match against the stored specification only. An unknown query parameter,
or a value the implementation does not support, is rejected rather than ignored.

The request example:

```text
GET /v1alpha/thumbnails?association_target_resource_address=https%3A%2F%2Faccount.blob.core.windows.net%2Fcontainer%2Fphotos%2Fimage.jpg&encoding_format=PNG HTTP/1.1
Authorization: Bearer <access-token>
```

The success response returns `200 OK`. Entries are ordered by the thumbnail's `resource_address` and **capped at
1000**; a target with more thumbnails has the remainder discarded, not paginated, so a response holding exactly
1000 entries may be incomplete. Addresses and identities are reported as stored in the association metadata.

```text
HTTP/1.1 200 OK
Content-Type: application/json

{
   "thumbnails": [
      {
         "target_type": "FILE",
         "association_target": {
            "resource_address": "https://account.blob.core.windows.net/container/photos/image.jpg",
            "resource_identity": "fileobject:src-def456:v3"
         },
         "thumbnail": {
            "resource_address": "https://account.blob.core.windows.net/container/thumbnails/8b6066de.../256x256/image.jpg.auto.png",
            "resource_identity": "fileobject:thumb-abc123:v7"
         },
         "specification": {
            "type": "IMAGE_TO_IMAGE",
            "width": 256,
            "height": 256,
            "encoding_format": "PNG",
            "transform_type": "FIT",
            "crop_strategy": "CENTER",
            "quality": 85,
            "preserve_aspect_ratio": true
         },
         "create_time": "2026-09-11T12:34:56Z"
      }
   ]
}
```

A target with no associations, or none matching the filters, is a successful read of nothing:

```text
HTTP/1.1 200 OK
Content-Type: application/json

{
   "thumbnails": []
}
```

A stored entry that cannot be interpreted is skipped and logged; the readable entries are still returned. A
target whose every entry is unreadable therefore also answers `200 OK` with an empty list.

(thumbnail-api-rest-api-get-v1alpha-thumbnails-error-conditions)=
### Error conditions

The following error conditions are defined:

**Missing, malformed, invalid, or expired credential**

```text
HTTP/1.1 401 Unauthorized
Content-Type: application/json

{
   "source": "thumbnail-service",
   "error_message": "Unauthorized"
}
```

**The credential is valid but the caller is not authorized for the target**

```text
HTTP/1.1 403 Forbidden
Content-Type: application/json

{
   "source": "thumbnail-service",
   "error_message": "Forbidden"
}
```

**Invalid or unsupported query parameters**

```text
HTTP/1.1 400 Bad Request
Content-Type: application/json

{
   "source": "thumbnail-service",
   "error_message": "invalid query: width must be between 1 and 8192"
}
```

Other messages of this class: `invalid query: association_target_resource_address is required`,
`invalid query: association_target_resource_address must not exceed 2048 characters`, and `invalid query: ...`
naming an unknown parameter or an undefined enum value.

**The implementation's association store could not be read**

```text
HTTP/1.1 502 Bad Gateway
Content-Type: application/json

{
   "source": "thumbnail-service",
   "error_message": "Storage service is unavailable"
}
```

(thumbnail-api-rest-api-post-v1alpha-associations)=
## POST /v1alpha/associations

```text
POST /v1alpha/associations HTTP/1.1
Authorization: Bearer <accessToken>
Content-Type: application/json

{
   "target_type": "FILE" | "FOLDER",
   "thumbnail": <a Resource with "resource_address" and "resource_identity">,
   "association_target": <a Resource with "resource_address" and, for FILE, "resource_identity">,
   "specification": <a flat, "type"-tagged specification>
}
```

This API records that an already-existing thumbnail belongs to a target resource. It does not generate anything:
the caller has produced, or otherwise obtained, the thumbnail bytes and is registering the link.

`thumbnail` MUST carry both `resource_address` and `resource_identity` — a thumbnail is always a file.

`association_target` MUST carry `resource_address`. When `target_type` is `FILE` (the default when omitted) it
MUST also carry `resource_identity`; when `target_type` is `FOLDER` it MUST NOT.

`specification` is REQUIRED and MUST be a flat, `type`-tagged object (see
{ref}`Wire shapes <thumbnail-api-associations-and-resources-wire-shapes>`). `type` MUST be one of the
specification kinds defined in {ref}`Specifications <thumbnail-api-associations-and-resources-specifications>`,
and `encoding_format` MUST be a value of the corresponding encoding enumeration. An implementation MUST accept
every specification kind the type system defines, regardless of which kinds it can itself generate: recording
an association for a thumbnail the caller already produced needs no conversion capability. A `type` or
`encoding_format` the type system does not define MUST be rejected with `400 Bad Request` naming the field,
before anything is recorded.

`width` and `height` MUST be in `1..=8192`. For lossy encodings (`JPEG`, `WEBP`) `quality`, when present, MUST
be in `1..=100`; for lossless encodings an implementation MAY ignore `quality`. Omitted OPTIONAL fields of an
`IMAGE_TO_IMAGE` specification resolve to the implementation's configured defaults before the association is
recorded. A `null` `quality` is equivalent to omitting it.

The request example:

```text
POST /v1alpha/associations HTTP/1.1
Authorization: Bearer <access-token>
Content-Type: application/json

{
   "target_type": "FILE",
   "thumbnail": {
      "resource_address": "https://account.blob.core.windows.net/container/.thumbs/image-256.png",
      "resource_identity": "fileobject:thumb-abc123:v7"
   },
   "association_target": {
      "resource_address": "https://account.blob.core.windows.net/container/photos/image.jpg",
      "resource_identity": "fileobject:src-def456:v3"
   },
   "specification": {
      "type": "IMAGE_TO_IMAGE",
      "width": 256,
      "height": 256,
      "encoding_format": "PNG",
      "transform_type": "FIT",
      "crop_strategy": "CENTER",
      "quality": 85,
      "preserve_aspect_ratio": true
   }
}
```

Registering a pre-generated thumbnail on a folder:

```text
POST /v1alpha/associations HTTP/1.1
Authorization: Bearer <access-token>
Content-Type: application/json

{
   "target_type": "FOLDER",
   "thumbnail": {
      "resource_address": "https://account.blob.core.windows.net/container/projects/.folder-preview.png",
      "resource_identity": "fileobject:thumb-folder-9:v1"
   },
   "association_target": {
      "resource_address": "https://account.blob.core.windows.net/container/projects/"
   },
   "specification": {
      "type": "MANUAL",
      "width": 512,
      "height": 512,
      "encoding_format": "PNG"
   }
}
```

The success response returns `204 No Content` with an empty body. Callers MUST treat any `2xx` status as success
rather than matching `200`. The stored association — including the resolved specification and its deterministic
key — is readable through `GET /v1alpha/thumbnails`. Associating the same thumbnail with the same target and
specification again overwrites the same entry.

```text
HTTP/1.1 204 No Content
```

An implementation MAY publish a change notification once the association is recorded. If it does, the
notification MUST NOT be emitted before the association is readable through `GET /v1alpha/thumbnails`, and
publication MUST NOT affect the response: `204 No Content` is returned whether or not a notification was
delivered. Delivery is best-effort — a notification MAY be lost — so consumers MUST NOT rely on it as the only
signal that an association exists; the read API is authoritative.

(thumbnail-api-rest-api-post-v1alpha-associations-error-conditions)=
### Error conditions

The following error conditions are defined:

**Missing, malformed, invalid, or expired credential**

```text
HTTP/1.1 401 Unauthorized
Content-Type: application/json

{
   "source": "thumbnail-service",
   "error_message": "Unauthorized"
}
```

**The request is invalid — malformed JSON, a missing or blank address, a file without an identity, a folder with
an identity, a missing specification, or a value outside its range.** Nothing is written.

```text
HTTP/1.1 400 Bad Request
Content-Type: application/json

{
   "source": "thumbnail-service",
   "error_message": "invalid association_target resource: resource_identity is required for file resources"
}
```

Other messages of this class: `specification is required`,
`invalid thumbnail resource: resource_address is required and non-empty`,
`invalid association_target resource: resource_identity must be omitted for folder resources`,
`specification width and height must both be between 1 and 8192`,
`specification quality must be between 1 and 100 for JPEG/WebP`, and `invalid JSON body: ...`.

**Recording the association failed for a reason other than authentication.** If the second of the two writes
failed, the first has been rolled back; if the rollback itself failed, the message names both failures and the
entry that must be removed by hand.

```text
HTTP/1.1 500 Internal Server Error
Content-Type: application/json

{
   "source": "thumbnail-service",
   "error_message": "Failed to associate thumbnail: Storage service call failed: ..."
}
```

(thumbnail-api-rest-api-delete-v1alpha-associations)=
## DELETE /v1alpha/associations

```text
DELETE /v1alpha/associations?association_target_resource_address=<target address>&thumbnail_resource_address=<thumbnail address>[&target_type=FILE|FOLDER][&association_target_resource_identity=<identity>][&thumbnail_resource_identity=<identity>] HTTP/1.1
Authorization: Bearer <accessToken>
```

This API removes one thumbnail's association with one target resource. Other thumbnails associated with the
same target are left untouched, and the thumbnail resource itself remains for its owner to manage; nothing is
deleted from storage.

`association_target_resource_address` and `thumbnail_resource_address` are REQUIRED and MUST be non-blank. The
target address selects the metadata collection; the thumbnail address selects the single entry whose stored
`thumbnail.resource_address` matches it.

`target_type` is OPTIONAL and defaults to `FILE`. The two identity parameters are OPTIONAL and are ignored: only
addresses locate and remove entries, for `FILE` and `FOLDER` targets alike.

The request example:

```text
DELETE /v1alpha/associations?association_target_resource_address=https%3A%2F%2Faccount.blob.core.windows.net%2Fcontainer%2Fphotos%2Fimage.jpg&thumbnail_resource_address=https%3A%2F%2Faccount.blob.core.windows.net%2Fcontainer%2F.thumbs%2Fimage-256.png HTTP/1.1
Authorization: Bearer <access-token>
```

The success response returns `200 OK`. `succeeded` carries the removed thumbnail; `failed` is empty:

```text
HTTP/1.1 200 OK
Content-Type: application/json

{
   "succeeded": [
      {
         "resource_address": "https://account.blob.core.windows.net/container/.thumbs/image-256.png",
         "resource_identity": "fileobject:thumb-abc123:v7"
      }
   ],
   "failed": []
}
```

The operation is **idempotent**. When the thumbnail is not associated with the target — already removed, or
never linked — the response is `200 OK` with both arrays empty, never a not-found error:

```text
HTTP/1.1 200 OK
Content-Type: application/json

{
   "succeeded": [],
   "failed": []
}
```

An implementation MAY publish a change notification once the association is removed. If it does, the
notification MUST NOT be emitted before the removal is visible through `GET /v1alpha/thumbnails`, and
publication MUST NOT affect the response. Delivery is best-effort — a notification MAY be lost — so consumers
MUST NOT rely on it as the only signal that an association was removed; the read API is authoritative.

(thumbnail-api-rest-api-delete-v1alpha-associations-error-conditions)=
### Error conditions

The following error conditions are defined:

**Missing, malformed, invalid, or expired credential.** Nothing is removed.

```text
HTTP/1.1 401 Unauthorized
Content-Type: application/json

{
   "source": "thumbnail-service",
   "error_message": "Unauthorized"
}
```

**A required address is missing or blank, or `target_type` is not `FILE` or `FOLDER`.** Validation runs before
the storage backend is contacted.

```text
HTTP/1.1 400 Bad Request
Content-Type: application/json

{
   "source": "thumbnail-service",
   "error_message": "invalid thumbnail_resource_address: resource_address is required and non-empty"
}
```

**The association entry was found but could not be removed.** The response body is the
`DisassociateThumbnailResponse` shape, with `failed` identifying the thumbnail and the backend error.

```text
HTTP/1.1 502 Bad Gateway
Content-Type: application/json

{
   "succeeded": [],
   "failed": [
      {
         "resource_address": "https://account.blob.core.windows.net/container/.thumbs/image-256.png",
         "resource_identity": "fileobject:thumb-abc123:v7",
         "error": "failed to remove thumbnail from storage: Storage service call failed: ..."
      }
   ]
}
```

**The association store could not be read at all.** No entry was inspected; both arrays are
empty.

```text
HTTP/1.1 502 Bad Gateway
Content-Type: application/json

{
   "succeeded": [],
   "failed": []
}
```

(thumbnail-api-rest-api-interactive-api-reference)=
## OpenAPI definition

An OpenAPI 3.1 document generated from the reference implementation is kept in the `thumbnail-service`
repository. It describes what that implementation serves and may lag this specification.

(thumbnail-api-rest-api-see-next)=
## See next

* {ref}`gRPC specification <thumbnail-api-grpc-grpc-specification>`
