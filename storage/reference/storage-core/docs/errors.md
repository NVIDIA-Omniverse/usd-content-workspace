# Error Handling

This document describes the errors returned by each Storage Core gRPC RPC method and how backend HTTP errors are mapped to gRPC/HTTP status codes.

## Errors by RPC Method

### FileObject Service

---

### `Stat`

| gRPC Code | HTTP Status | Cause |
|---|---|---|
| `INVALID_ARGUMENT` | 400 | Invalid URL; address ends with `/` (is a folder, not a file) |
| `NOT_FOUND` | 404 | Object does not exist |
| `PERMISSION_DENIED` | 403 | Access denied at the backend |
| `FAILED_PRECONDITION` | 412 | ETag or if-match condition not met |
| `DEADLINE_EXCEEDED` | 408 | Request timed out |
| `INTERNAL` | 500 | SDK construction or dispatch failure |
| `UNAVAILABLE` | 503 | Backend returned 503 |

---

### `Read` / `ReadFromAddress`

| gRPC Code | HTTP Status | Cause |
|---|---|---|
| `INVALID_ARGUMENT` | 400 | Invalid URL; invalid download preference; ETag or content-length mismatch on response |
| `NOT_FOUND` | 404 | Object does not exist |
| `PERMISSION_DENIED` | 403 | Access denied at the backend; SAS credential issue (Azure only) |
| `FAILED_PRECONDITION` | 412 | ETag or if-match condition not met |
| `DEADLINE_EXCEEDED` | 408 | Request timed out |
| `INTERNAL` | 500 | SDK construction or dispatch failure; presigned URL generation failure (S3); SAS token generation failure (Azure) |
| `UNAVAILABLE` | 503 | Backend returned 503 |

Responses are streamed. The first message returns resource info or an error; subsequent messages return data chunks or a redirect URL. Each chunk in the stream can independently error.

---

### `Enumerate`

| gRPC Code | HTTP Status | Cause |
|---|---|---|
| `INVALID_ARGUMENT` | 400 | Invalid URL; bad list parameters |
| `NOT_FOUND` | 404 | Empty listing at a non-top-level path |
| `PERMISSION_DENIED` | 403 | Access denied at the backend |
| `DEADLINE_EXCEEDED` | 408 | Request timed out |
| `INTERNAL` | 500 | SDK error |
| `UNAVAILABLE` | 503 | Backend returned 503 |

Responses are streamed. Each page in the stream can independently error.

---

### `FetchWriteTypeInfo`

| gRPC Code | HTTP Status | Cause |
|---|---|---|
| `INVALID_ARGUMENT` | 400 | Invalid URL; versioned address not allowed |

No backend SDK call is made. Returns static configuration based on size thresholds.

---

### `Write`

| gRPC Code | HTTP Status | Cause |
|---|---|---|
| `INVALID_ARGUMENT` | 400 | First stream message is not `Params`; subsequent messages are not `Chunk`; invalid URL; versioned address not allowed; invalid upload preference |

#### Body upload (direct write)

| gRPC Code | HTTP Status | Cause |
|---|---|---|
| `NOT_FOUND` | 404 | Bucket or container does not exist |
| `PERMISSION_DENIED` | 403 | Access denied |
| `FAILED_PRECONDITION` | 412 | ETag mismatch on conditional write |
| `ALREADY_EXISTS` | 409 | Conditional write conflict |
| `DEADLINE_EXCEEDED` | 408 | Request timed out |
| `INTERNAL` | 500 | SDK error |
| `UNAVAILABLE` | 503 | Backend returned 503 |

#### Redirect upload (presigned/SAS URL returned to caller)

| gRPC Code | HTTP Status | Cause |
|---|---|---|
| `INTERNAL` | 500 | Presigning config failure (S3); SAS generation failure (Azure) |
| `PERMISSION_DENIED` | 403 | SAS credential issue (Azure only) |

#### Multipart upload (initiated by the service)

| gRPC Code | HTTP Status | Cause |
|---|---|---|
| `INTERNAL` | 500 | CreateMultipartUpload failure; part presign failure |

Plus all body upload errors on the initial CreateMultipartUpload call.

---

### `UploadPart`

| gRPC Code | HTTP Status | Cause |
|---|---|---|
| `INVALID_ARGUMENT` | 400 | `part_count` is 0; invalid address |
| `INTERNAL` | 500 | Presign/SAS generation failure; client creation failure |
| `PERMISSION_DENIED` | 403 | Azure SAS credential issue |

---

### `CompleteMultipartUpload`

| gRPC Code | HTTP Status | Cause |
|---|---|---|
| `INVALID_ARGUMENT` | 400 | Invalid address; missing ETag header in parts; bad upload ID |
| `NOT_FOUND` | 404 | Upload not found (`NoSuchUpload`) |
| `FAILED_PRECONDITION` | 412 | Precondition failed |
| `ALREADY_EXISTS` | 409 | Conditional conflict |
| `PERMISSION_DENIED` | 403 | Access denied |
| `DEADLINE_EXCEEDED` | 408 | Request timed out |
| `INTERNAL` | 500 | Server error |
| `UNAVAILABLE` | 503 | Backend returned 503 |

S3 can return errors inside an HTTP 200 response body for this call; the service parses the response body and maps error codes accordingly.

---

### `AbortMultipartUpload`

| gRPC Code | HTTP Status | Cause |
|---|---|---|
| `INVALID_ARGUMENT` | 400 | Invalid address |
| `NOT_FOUND` | 404 | Upload not found |
| `PERMISSION_DENIED` | 403 | Access denied |
| `DEADLINE_EXCEEDED` | 408 | Request timed out |
| `INTERNAL` | 500 | Server error |
| `UNAVAILABLE` | 503 | Backend returned 503 |

Azure: this is a no-op that returns success immediately. Uncommitted blocks are garbage collected by Azure.

---

### `CompleteRedirectUpload`

| gRPC Code | HTTP Status | Cause |
|---|---|---|
| `INVALID_ARGUMENT` | 400 | Invalid address; missing ETag or LastModified response headers |
| `INTERNAL` | 500 | LastModified timestamp parse failure (Azure) |

No backend SDK call is made. Validates response headers from the redirect upload.

---

### `Delete`

| gRPC Code | HTTP Status | Cause |
|---|---|---|
| `INVALID_ARGUMENT` | 400 | Invalid URL; versioned address not allowed; invalid previous_version |
| `NOT_FOUND` | 404 | Object does not exist (S3 only; Azure treats delete of non-existent object as success) |
| `PERMISSION_DENIED` | 403 | Access denied |
| `FAILED_PRECONDITION` | 412 | ETag mismatch on conditional delete |
| `ALREADY_EXISTS` | 409 | Conditional conflict |
| `DEADLINE_EXCEEDED` | 408 | Request timed out |
| `INTERNAL` | 500 | Server error |
| `UNAVAILABLE` | 503 | Backend returned 503 |

---

### `Copy`

| gRPC Code | HTTP Status | Cause |
|---|---|---|
| `INVALID_ARGUMENT` | 400 | Invalid source or destination URL; versioned destination not allowed; key mismatch |
| `NOT_FOUND` | 404 | Source object does not exist |
| `PERMISSION_DENIED` | 403 | Access denied |
| `FAILED_PRECONDITION` | 412 | ETag mismatch on source or destination |
| `ALREADY_EXISTS` | 409 | Conditional conflict |
| `DEADLINE_EXCEEDED` | 408 | Request timed out |
| `INTERNAL` | 500 | SDK construction error; copy result missing; timestamp parse failure |
| `UNAVAILABLE` | 503 | Backend returned 503 |

---

### `Move` (v1alpha only)

All Copy errors, plus:

| gRPC Code | HTTP Status | Cause |
|---|---|---|
| `NOT_FOUND` | 404 | Source is a folder (address ends with `/`) |
| `INVALID_ARGUMENT` | 400 | Versioned source address not allowed |

Move is implemented as copy + delete. If copy succeeds but delete fails with `NOT_FOUND` or `FAILED_PRECONDITION`, the move still returns success with the copy result. Other delete errors are propagated.

---

### FileFolder Service

---

### `List` / `ListStat`

| gRPC Code | HTTP Status | Cause |
|---|---|---|
| `INVALID_ARGUMENT` | 400 | Missing `folder` field; invalid URL; versioned address not allowed |
| `NOT_FOUND` | 404 | Empty listing at a non-top-level path |
| `PERMISSION_DENIED` | 403 | Access denied |
| `DEADLINE_EXCEEDED` | 408 | Request timed out |
| `INTERNAL` | 500 | SDK error |
| `UNAVAILABLE` | 503 | Backend returned 503 |

Responses are streamed. Each page can independently error. ListStat silently skips items with malformed resource identities.

---

### `CreateFolder` (v1alpha only)

| gRPC Code | HTTP Status | Cause |
|---|---|---|
| `INVALID_ARGUMENT` | 400 | Missing `folder` field; invalid URL |
| `FAILED_PRECONDITION` | 412 | A non-folder object already exists at the path |
| `PERMISSION_DENIED` | 403 | Access denied |
| `INTERNAL` | 500 | SDK error |
| `UNAVAILABLE` | 503 | Backend returned 503 |

If the folder already exists (412 from backend), the call returns success.

---

### `DeleteFolder`

| gRPC Code | HTTP Status | Cause |
|---|---|---|
| `INVALID_ARGUMENT` | 400 | Missing `folder` field; invalid URL; a non-folder object exists at the path |
| `PERMISSION_DENIED` | 403 | Access denied |
| `DEADLINE_EXCEEDED` | 408 | Request timed out |
| `INTERNAL` | 500 | SDK error |
| `UNAVAILABLE` | 503 | Backend returned 503 |

---

### `GetFolderMode` (v1alpha only)

No errors. Always returns `FolderMode::Hybrid`.

---

### Capabilities Service

---

### `ListServices`

No errors. Returns a static service list.

---

### `ListTopLevelAddresses`

| gRPC Code | HTTP Status | Cause |
|---|---|---|
| `INVALID_ARGUMENT` | 400 | URL parse failure (Azure only) |
| `PERMISSION_DENIED` | 403 | Access denied (Azure only) |
| `DEADLINE_EXCEEDED` | 408 | Request timed out (Azure only) |
| `INTERNAL` | 500 | SDK error (Azure only) |
| `UNAVAILABLE` | 503 | Backend returned 503 (Azure only) |

S3: reads from configuration only, no backend call, no errors possible.

---

### `ListRoutes` (v1alpha only)

No errors. Returns a static route list.

---

### Versioning Service

---

### `EnumerateVersions`

| gRPC Code | HTTP Status | Cause |
|---|---|---|
| `INVALID_ARGUMENT` | 400 | Invalid URL; bad address format |
| `NOT_FOUND` | 404 | Object does not exist |
| `PERMISSION_DENIED` | 403 | Access denied |
| `DEADLINE_EXCEEDED` | 408 | Request timed out |
| `INTERNAL` | 500 | SDK error; client creation failure |
| `UNAVAILABLE` | 503 | Backend returned 503 |

Responses are streamed. Each page can independently error.

---

## Error Code Reference

### gRPC and HTTP Status Code Mapping

Both S3 and Azure backends use an identical `SdkError` enum with the following mappings:

| gRPC Code | HTTP Status | Description |
|---|---|---|
| `INVALID_ARGUMENT` (3) | 400 Bad Request | Invalid URL, missing required fields, bad parameters |
| `DEADLINE_EXCEEDED` (4) | 408 Request Timeout | Request timed out at the backend |
| `NOT_FOUND` (5) | 404 Not Found | Object, container, or upload does not exist |
| `ALREADY_EXISTS` (6) | 409 Conflict | Conditional write conflict (e.g. object already exists) |
| `PERMISSION_DENIED` (7) | 403 Forbidden | Authentication or authorization failure |
| `FAILED_PRECONDITION` (9) | 412 Precondition Failed | ETag or if-match condition not met |
| `OUT_OF_RANGE` (11) | 416 Range Not Satisfiable | Invalid byte range request |
| `UNIMPLEMENTED` (12) | 501 Not Implemented | Operation not supported by the backend |
| `INTERNAL` (13) | 500 Internal Server Error | Server-side failure, SDK construction error, parse error |
| `UNAVAILABLE` (14) | 503 Service Unavailable | Backend temporarily unavailable or throttling |
| `UNKNOWN` (2) | 500 Internal Server Error | Unmapped or unexpected error from the backend |

### Backend HTTP Status to SdkError Mapping

#### S3 (AWS)

| HTTP Status | SdkError | Notes |
|---|---|---|
| 400 | InvalidArgument | |
| 403 | AccessDenied | |
| 404 | NotFound | |
| 405 | AccessDenied | Method not allowed treated as access denied |
| 409 | Conflict | |
| 411 | InvalidArgument | Missing content-length |
| 412 | PreconditionFailed | |
| 416 | InvalidRange | |
| 500 | Internal | |
| 501 | Unimplemented | |
| 503 | Unavailable | Includes `SlowDown` throttling |
| Timeout | Timeout | SDK-level timeout |
| ConstructionFailure | Internal | SDK client setup error |
| DispatchFailure | Internal | HTTP dispatch error |

S3 `CompleteMultipartUpload` can return errors inside an HTTP 200 response body. These are parsed by error code (`NoSuchKey`, `NoSuchUpload`, `ConditionalRequestConflict`, `InternalError`, `SlowDown`, etc.) and mapped to the corresponding `SdkError` variant.

#### Azure

| HTTP Status | SdkError | Notes |
|---|---|---|
| 400 | InvalidArgument | |
| 401 | AccessDenied | |
| 403 | AccessDenied | |
| 404 | NotFound | |
| 407 | AccessDenied | Proxy authentication required |
| 408 | Timeout | |
| 409 | PreconditionFailed | Azure maps 409 to precondition, not conflict |
| 410 | NotFound | Gone treated as not found |
| 412 | PreconditionFailed | |
| 413 | InvalidRange | Payload too large |
| 414, 415, 422 | InvalidArgument | Various request format issues |
| 416 | InvalidRange | |
| 423 | Unavailable | Resource locked |
| 429 | Internal | Too many requests (not mapped to Unavailable) |
| 500 | Internal | |
| 501, 502 | Internal | |
| 503 | Unavailable | |
| 504 | Timeout | Gateway timeout |
| 511 | AccessDenied | Network authentication required |
| Credential error | AccessDenied | SDK credential failure |
| I/O error | Internal | |

### Service-Level Errors

These errors originate within Storage Core itself, not from the backend SDK:

| Error Source | gRPC Code | When |
|---|---|---|
| URL parse failure | `INVALID_ARGUMENT` | Malformed resource address |
| Validation failure | `INVALID_ARGUMENT` | Missing fields, versioned address where not allowed |
| Presigning config | `INTERNAL` | Failed to build presigned URL config (S3) |
| SAS generation | `INTERNAL` | Failed to generate SAS token (Azure) |
| Timestamp parse | `INTERNAL` | Failed to parse LastModified or other timestamps |
| Byte stream error | `INTERNAL` | AWS byte stream read failure |
| System time error | `INTERNAL` | System clock error |
| JSON parse error | `INVALID_ARGUMENT` (S3) / `INTERNAL` (Azure) | Malformed JSON in request or response |

### Layer Errors

When caching or middleware layers are enabled, they can wrap backend errors or produce their own:

| Layer | Possible Errors |
|---|---|
| stat_cache | `INTERNAL` (cache errors); all backend errors passed through |
| list_cache | `INVALID_ARGUMENT`, `NOT_FOUND`, `INTERNAL`; all backend errors passed through |
| small_object_cache | `INVALID_ARGUMENT`, `INTERNAL` (cache/internal errors); all backend and stream errors passed through without being retained |
| manifest | `INVALID_ARGUMENT` (bad entry, unsupported format), `NOT_FOUND` (object path), `PERMISSION_DENIED` (method not allowed), `INTERNAL` |
| additional_headers | `INVALID_ARGUMENT` (metadata conversion errors) |

The small-object cache may share an error with callers already waiting on the
same in-progress fill, but it does not store that error for a later request. Its
legacy `errors` configuration field is accepted but has no effect.
