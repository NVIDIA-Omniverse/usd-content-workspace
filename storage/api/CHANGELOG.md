# 1.0.0-beta.6

* Added v1alpha native storage metadata to FileObject `Write`, `Read`, `Stat`, and FileFolder `ListStat`, including a capability query that clients can use before writing native metadata. The reference filesystem service persists this metadata per object version across direct, redirect, and multipart writes.
* Added v1alpha `IsVersioned` (VersioningService) and `IsReadOnly` (FileObjectService) capability queries for any resource address. The reference filesystem service now supports versioned, unversioned, and read-only modes, with dedicated conformance coverage.
* Expanded the v1alpha Metadata API with namespace-aware `GetMetadataBatch`, `DeleteMetadataBatch`, and `QueryMetadata` operations. The existing single-item REST endpoints remain available for default-namespace access but are now deprecated.
* **Breaking (v1alpha):** Redesigned `UpdateMetadataBatch` request and response entries to use `ResourceLocation`, support per-entry namespaces, and use the shared batch error model introduced for the new batch operations.
* Defined FileObject `Enumerate` in v1alpha and v1beta to recursively return every data object below the requested enumerable address, including descendants at any depth.
* Clarified v1alpha `ListRoutes` wildcard matching and deterministic route selection: `*` matches one segment, trailing `**` matches descendants, and the most specific route wins with a lexicographic tie-break.
* Expanded conformance coverage to require identities returned by `Write`, redirect completion, and `Copy` to remain bound to the original bytes on versioned storage. On unversioned storage, superseded identities must return `NOT_FOUND`/404 rather than resolve to replacement bytes.
* Fixed the reference filesystem service's optimistic writes so `previous_version` is checked atomically at commit time for direct, redirect, and multipart uploads.
* Tightened the reference Metadata API's `resource_identity` validation. Batch operations now return per-entry `INVALID_ARGUMENT` for invalid identities, while `QueryMetadata` returns request-level `INVALID_ARGUMENT`/HTTP 400.
* Fixed published OpenAPI specifications whose `$ref` keys were stripped by the OSS bundling step, producing invalid YAML.

# 1.0.0-beta.5

* Added `fail_if_exists` parameter to `WriteParameters`, `CopyRequest`, and `MoveRequest` of the FileObject API.
* Added `UpdateMetadataBatch` to the Metadata API.

# 1.0.0-beta.4

* Improved local filesystem example startup defaults so backend environment variables (including `FILESERVICE_STATIC_DIR`) are honored when no backend subcommand is provided.
* Updated local filesystem example Docker defaults and documentation to keep backend startup behavior consistent.
* Improved REST conformance tests for compatibility with newer FastAPI behavior, including metadata request handling and write/upload flow checks.
* Updated documentation and OSS release packaging to include both `v1beta` and `v1alpha` API specifications.

# 1.0.0-beta.2

* Added a reference implementation of the Storage API using Python and the local filesystem as an example.
* Added a conformance test suite documenting behavioral expectations of any Storage API implementation using Gherkin BDD,
and a runnable implementation in Python that can be used to test any storage service for specification adherence.
* Now including a v1alpha channel as a preview of functions that might make it into v1beta.
  * The FileObjectService support copy and move in the v1alpha version.
  * The FileFolderService supports CreateFolder and GetFolderMode in v1alpha, allowing for different handling of empty folders to be exposed by a service implementation.
  * The CapabilitiesService got an additional function, ListRoutes, that can be used  in setups with more than one storage service instance serving different resource addresses.
  * The VersioningService adds experimental support for versioned resource addresses in addition to resource identities.
  * Added a new MetadataService in v1alpha, which can be used to store arbitrary metadata on resource addresses or resource identities. 

# 1.0.0-beta.1

Initial closed availability release.
