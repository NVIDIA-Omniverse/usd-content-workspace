(thumbnail-api-index-omniverse-storage-thumbnail-api)=
# Omniverse Storage Thumbnail API

(thumbnail-api-index-introduction)=
## Introduction

This document describes the Thumbnail API: the interface through which Omniverse applications and services
request preview images for storage resources and record, query, and remove the **associations** between a
source resource and its thumbnails.

It is an API design and specification, together with recommendations for implementing it — not a prescription
of one architecture. You should be able to take this document and the conformance test suite, build a
thumbnail service with your own generation logic, and verify that clients written against this API work with
it. Requirements are stated with RFC 2119 keywords: a `MUST` is a requirement every conformant implementation
meets — the conformance suite checks those it can observe from outside — and a `SHOULD` or `MAY` is a
recommendation or an allowed choice. Where this document mentions how NVIDIA's **reference implementation**
behaves, that is illustration, not requirement; its behavior is documented separately in the Thumbnail Service
documentation.

The following sections describe what triggers thumbnail generation and what an accepted request guarantees,
the resource and association model, and the REST and gRPC specifications.

```{toctree}
:maxdepth: 1
:caption: Contents:
ecosystem-usage
thumbnail-generation
associations-and-resources
rest-api
grpc
```

(thumbnail-api-index-assumptions-constraints-dependencies)=
## Assumptions, Constraints, Dependencies

* Callers authenticate with a bearer token, issued by an OpenID Connect Identity Provider in the reference
  deployment. How an implementation validates that token is not part of this specification: it MAY verify the
  token itself (signature, expiry, audience) before any other logic runs, MAY delegate the authorization decision
  to the storage backend it reads and writes, or MAY combine both. Whatever the strategy, an implementation MUST
  refuse a request that carries no usable credential before acting on it, MUST report a missing, invalid, or
  expired token as an authentication failure (`401` / `UNAUTHENTICATED`), and MUST report a valid token that is
  not authorized for the resource as a permission failure (`403` / `PERMISSION_DENIED`).
* Thumbnail generation MAY complete **synchronously** — the response to a generation request is returned once
  the thumbnails exist and their associations are readable — or **asynchronously** — the response acknowledges
  that the request was accepted and the thumbnails appear later. A client MUST NOT assume either mode. In both,
  the client discovers thumbnails through the association read API, and through change notifications where an
  implementation publishes them.
* Every source resource and every thumbnail is addressed by a storage `resource_address`. Files additionally
  carry an opaque `resource_identity`; folders do not.
* The Thumbnail API is generic over what a thumbnail is made from and what it is made into. A **specification**
  names the **processor** — the kind of conversion, such as image-to-image, video-to-image, video-to-video,
  USD-to-image, or USD-to-video — together with its parameters; `MANUAL` registers a thumbnail the caller
  produced. The API does not limit which processors, source formats, or output encodings a thumbnail service
  supports: that is a property of each implementation. An implementation MUST reject a generation request for a
  processor, format, or encoding it does not support with `400` / `INVALID_ARGUMENT` before accepting any work,
  and MUST NOT silently substitute another.
* HTTP and gRPC are two transports over one API: equivalent requests are validated identically and produce
  identical stored metadata and events.

(thumbnail-api-index-definitions-acronyms-abbreviations)=
## Definitions, Acronyms, Abbreviations

| Term                 | Definition                                                                                                    |
|----------------------|---------------------------------------------------------------------------------------------------------------|
| Resource             | An object in the storage backend identified by `resource_address` and, for files, `resource_identity`        |
| Resource address     | The storage URI of a resource, for example `https://account.blob.core.windows.net/container/image.jpg`      |
| Resource identity    | An opaque string, assigned by the storage backend, that — together with the resource's `resource_address` — identifies one exact version of that resource. It never identifies a resource on its own; only `resource_address` is used to locate one. |
| Association          | The recorded link between an association target and one thumbnail, together with the specification that produced it |
| Association target   | The file or folder a thumbnail belongs to (`target_type` `FILE` or `FOLDER`)                                  |
| Specification        | The processor that produced, or will produce, a thumbnail and its parameters — for image-to-image: dimensions, encoding format, transform, crop strategy, quality |
| Destination          | The exact, absolute address in the storage backend a generated thumbnail is written to                        |
| Storage backend      | The system that holds source resources and thumbnails and that an implementation reads from and writes to. Any backend that addresses objects by a stable URI qualifies — an object store such as Amazon S3, Azure Blob Storage, or Google Cloud Storage, or a file system exposed over HTTP |
| IdP                  | Identity Provider                                                                                             |
| AuthN                | Authentication                                                                                                |

(thumbnail-api-index-architecture-details)=
## Architecture Details

The Thumbnail API defines how clients request thumbnails for storage resources and how they discover the
thumbnails that exist. It is an API specification, not a deployment design: any service that implements the
REST and gRPC surfaces described here — both transports are required — and passes the conformance test suite is a
Thumbnail API.

The specification is organized around three groups of operations:

1. **Generating thumbnails** — generation is triggered by a client request or, where an implementation
   supports it, by events from other systems; both are described in
   {ref}`Thumbnail generation <thumbnail-api-thumbnail-generation-thumbnail-generation>`. A client names a source resource by `resource_address`
   and, optionally, the specifications it wants. The implementation resolves defaults for anything omitted and
   produces one thumbnail per specification. An implementation MAY complete the work before it answers
   (synchronous completion) or accept the request and finish later (asynchronous completion); the response
   rules for both modes are defined on the same page.
2. **Managing associations** — an **association** records that a thumbnail belongs to an association target
   (a file or a folder), together with the specification that produced it. An implementation MUST record an
   association for every thumbnail it generates; clients can register thumbnails they produced themselves or
   remove a single association.
3. **Reading associations** — clients list the thumbnails associated with a target, filtered by
   specification, without knowing how or where the implementation stores them.

Two principles hold across all three groups:

- **Thumbnails are storage resources.** Every source and every thumbnail is addressed by a storage
  `resource_address`; files additionally carry an opaque `resource_identity`, folders do not. The Thumbnail API
  never serves image bytes — clients fetch a thumbnail from storage using the address an association returns.
- **The transport does not change the specification.** HTTP and gRPC expose the same operations, validate requests
  identically, and report failures with the equivalent status codes listed in {ref}`REST API <thumbnail-api-rest-api-rest-api>` and
  {ref}`gRPC specification <thumbnail-api-grpc-grpc-specification>`.
- **The API never carries image bytes.** Sources are read and thumbnails are written through the storage
  backend's own data plane, and clients fetch a thumbnail from the address an association returns. What that
  data plane is depends on the backend: an implementation over an object store might use the provider SDK or
  presigned URLs; one over a file system might read and write paths directly. Whichever it is, every
  `resource_address` the API returns MUST be a URI that the backend can resolve to the thumbnail bytes.

The Thumbnail API deliberately does not expose a task or job to poll, and does not offer an operation to delete
a thumbnail blob: removing an association leaves the thumbnail resource in place for its owner to manage.
Implementations MAY publish notifications when associations change.

(thumbnail-api-index-see-next)=
### See next

* {ref}`Ecosystem usage <thumbnail-api-ecosystem-usage-ecosystem-usage>`
* {ref}`Thumbnail generation <thumbnail-api-thumbnail-generation-thumbnail-generation>`
* {ref}`Resources, associations, and specifications <thumbnail-api-associations-and-resources-resources-associations-and-specifications>`
* {ref}`REST API <thumbnail-api-rest-api-rest-api>`
* {ref}`gRPC specification <thumbnail-api-grpc-grpc-specification>`
