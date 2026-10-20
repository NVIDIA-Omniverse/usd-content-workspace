# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.11.0-alpha.3]

### Changed

- **Breaking (configuration): the S3 and Azure backends keep TLS certificate and hostname verification on for the UCC proxy unless `UCC_PROXY_SSL_VERIFYCERT` is exactly `insecure`.** `false` and `0` used to turn it off; they now keep it on and log a warning. Deployments that reach a proxy with a self-signed certificate through `false` or `0` must switch to `insecure`


## [0.11.0-alpha.2]

### Changed

- **Breaking: `Versioning::is_versioned` takes the whole `Request<IsVersionedRequest>`**, like every other storage trait method, so the caller's metadata and layer extensions reach every layer and the backend; the manifest layer's cold `.manifest` read now carries the caller's metadata and tracing headers. The v1beta method, which no handler reached, is removed
- In strict mode (`allow_unconfigured_endpoints = false`), `IsVersioned` rejects an endpoint with no secrets entry and no backend-wide `versioning_enabled` with `NOT_FOUND`, as `IsReadOnly` does


## [0.11.0-alpha.1]

### Added

- OMPE-90246 expose native metadata through `FileObject`: `native_metadata` on stat/read/write, the `native_metadata_enabled` and `native_metadata_list_fanout_enabled` backend settings, and a startup consistency check between them and the caching layers
- OMPE-104659 `FileObject.IsReadOnly` and `Versioning.IsVersioned` capability RPCs, resolved from configuration with no backend call
- A per-endpoint `read_only` override (`READ_ONLY` secrets entry, backend-wide `s3.read_only` / `azure.read_only`, and an S3-only `READ_ONLY` environment fallback for unconfigured endpoints)


### Changed

- **`ServiceInfoProvider::service_info` now returns `Option<ServiceInfo>` and `list_in_services` is gone.** A service that is not part of the storage capability surface returns `None` instead of inventing an entry
- `IsReadOnly` and `IsVersioned` reject endpoints that strict mode (`allow_unconfigured_endpoints = false`) refuses to serve, rather than reporting a state for an address no operation can use
- `VERSIONING_ENABLED` is now honoured by the versioning resolver; previously it was parsed into the client but never reflected in the answer


### Security

- Deployments can opt out of serving the API documentation (`without_swagger_ui()`); this removes every `openapi.json` as well as the Swagger UI, since a machine-readable schema enumerates the API surface just as well
- Update `h2` to 0.4.19 on the request-serving path (RUSTSEC-2026-0258)


## [0.10.0]


### Changed

- Update packages
- Manifest layer resolver: second downstream service for the manifest layer's own internal reads
- Relicense to Apache-2.0


### Fixed

- OMPE-104009 canonicalize resource identity URL paths


## [0.9.0]


### Added

- Let services opt out of the storage capabilities ListServices


### Changed

- **small-object-cache:** Replace replay coalescing with moka single-flight


### Fixed

- Do not cache Absent manifest state
- Respect CacheControl::NoCache in list cache FileObject stat/read


## [0.8.2]


### Added

- OMPE-98507 FAILED_PRECONDITION for EnumerateVersions when versioning is configured off


## [0.8.1]


### Fixed

- OMPE-92921 block access to unconfigured S3/Azure endpoints


## [0.8.0]


### Added

- OMPE-96515 workspace manifest overlay (.wkspc/.manifest)
- `Versioning::is_versioning_enabled(top_level_address)` capability (v1alpha and v1beta) that reports whether a bucket/account has versioning enabled, answered from the operator-specified configuration: a per-endpoint `VERSIONING_ENABLED` secrets entry, falling back to the backend-wide `s3.versioning_enabled` / `azure.versioning_enabled` config (unset means enabled; no backend call, no runtime detection); manifest-backed workspace entries report enabled regardless of the physical backend.


### Changed

- Update LICENSE to Apache 2.0 and copyrights in preparation for github
- Refactor tests to use use nextest profiles


### Fixed

- Pool Redis L2 connections and raise default command timeout
- Preserve trailing whitespace in object names


## [0.7.2]


### Fixed

- OMPE-95382 replace mpsc-channel streaming with lazy async_stream BoxStreams to bound memory
- Clean up small_object_cache in-flight entry on terminal stream events
- Latch ListObjectVersions 403/AccessDenied per Client
- Short-circuit `EnumerateVersions` with `FAILED_PRECONDITION` / HTTP 412 when the operator configures versioning off for the endpoint (per-endpoint `VERSIONING_ENABLED` secrets entry, falling back to the backend-wide `s3.versioning_enabled` / `azure.versioning_enabled` config), avoiding confusing 403 errors from S3 on non-versioned buckets. Versioning state is specified manually — there is no runtime detection; unconfigured means versioning is treated as enabled (the legacy behavior).


## [0.7.1]


### Fixed

- OMPE-94624 strip query parameters from resource addresses when looking up address in list cache


## [0.7.0]


### Added

- OMPE-94177 update storage protos to 0.28.0
- Add production boxed builders and shared metrics state
- OMPE-89612 Consistent Resource IDs


### Fixed

- Validate Azure container names before slicing
- Fix OMPE-93491 The L2 cache changes can serve stale data after prefix invalidations and can leave local stale entries


## [0.6.0]


### Added

- OMPE-84310 implement second layer cache using redis/valkey
- OMPE-92431 cancel tasks
- OMPE-92558 Add configurable metric prefix
- OMPE-84456 Add metadata api trait, implementations, and cache


### Fixed

- Make azure pool_max_idle_per_host configurable


## [0.5.12]


### Added

- OMPE-89732 pass explicit credentials in secrets


## [0.5.11]


### Changed

- Update crates
- Clean up misleading and unhelpful log messages
- Remove unicode normalization
- Cargo deny fix for https://rustsec.org/advisories/RUSTSEC-2026-0098


## [0.5.10]


### Added

- OMPE-87307 add structured error details for gRPC and REST
- OMPE-88199 secrets hot-reload via file polling
- OMPE-69428 Add env var-based HTTP headers to AdditionalHeaders layer
- OMPE-30119 Add S3 rate-limit metrics via SDK interceptor
- OMPE-50888 Logging improvements based on Omniverse Logging SRD


### Changed

- Migrate all tracing macros to rate-limited variants


### Fixed

- OMPE-89547 missed url encoding for copy/move with s3 backend


## [0.5.9]


### Fixed

- OMPE-87288 Add protocol to routes returned from Azure backend
- OMPE-87143 write() should invalidate entire path in list-cache, not just the parent


## [0.5.8]


### Changed

- Add update rust crates skill


### Fixed

- OMPE-74603 Validate object keys prior to performing backend requests
- Update rustls-webpki to address RUSTSEC-2026-0049


## [0.5.7]


### Added

- OMPE-85056 add support for listroutes with GCP endpoints, legacy S3 urls, and azure custom domains


## [0.5.6]


### Added

- OMPE-84818 Add Service Principal auth to Azure backend
- OMPE-84820 support older deprecated S3 urls <bucket>.s3-<region>.amazonaws.com


### Fixed

- OMPE-83732 do not drop https:// scheme from S3 presigned urls when the UCC_PROXY is configured


## [0.5.5]


### Added

- OMPE-82280 add nginx cache to integration tests


### Changed

- Update crates. update dev/debugging build profiles to try and reduce build/link times


### Fixed

- Only rewrite CDN urls for presigned urls. do not force small objects to be redirects
- OMPE-82594 add read_versions_cdn() integration tests for S3/Azure for reading versioned files from CDN (CloudFront/FrontDoor)
- OMPE-82280 allow invalid self-signed SSL certs
- OMPE-82594 preserve any query parameters when rewriting cloudfront urls
- OMPE-82441 url encode resource addresses in responses from list cache


## [0.5.4]


### Fixed

- Add missing serde feature to Url crate dependency


## [0.5.3]


### Added

- OMPE-79307 add cloudfront support to s3 backend.
- OMPE-80825 Allow using Azure FrontDoor for reads


### Changed

- Add example grafana dashboard and traffic generator example


### Fixed

- OMPE-80756 mask sensitive gRPC headers in debug output.


## [0.5.2]


### Changed

- Updated the following local packages: storage-core-util


## [0.5.1]


### Changed

- Updated the following local packages: storage-core-util


## [0.5.0]


### Added

- OMPE-78424: Change observability stack to OpenTelemetry


### Changed

- Update all dependencies to the latest versions


## [0.4.4]


### Added

- OMPE-77492 Add source_location to all errors
- OMPE-69308 Add Manifest Layer


## [0.4.3] - 2026-01-14

### Added
- OMPE-75559: AWS AssumeRole / STS / IRSA support for S3 backend.

## [0.4.2] - 2025-12-12

### Fixed
- OMPE-75026: list and stat caches read_from_address() should skip performing stat() if item is already cached in the request extensions.

## [0.4.1] - 2025-12-11

### Fixed
- OMPE-74954: Configuration timeouts.keepalive is now optional.

## [0.4.0]

### Added
- OMPE-69254: Enable HTTP/2 feature for UCC_PROXY.
- OMPE-69505: Add retry policy configuration to backends. Increase retry defaults for UCC_PROXY.

## [0.3.5] - 2025-12-04

### Added
- OMPE-64175: invalidate_cache_entry_by_object_url() method for other layers to be able to invalidate cache entry.

## [0.3.4] - 2025-12-03

### Fixed
- OMPE-64175: Revert changes that removed version id from write, copy, move results.

## [0.3.3] - 2025-12-01

### Addded
- OMPE-64175: List cache layer. Replaces stat cache layer, caching results for stat(), list(), and list_stat().
- Updated storage-core-builder to 0.3.2
- Updated storage-core-util to 0.3.2
- Updated storage-core-protos to 0.25.0

### Fixed
- OMPE-69996: Various fixes to match the expected behavior of the conformance tests.

## [0.3.2] - 2025-11-04

### Added
- OMPE-70304: Conditional copy support for S3.

### Fixed
- OMPE-70306: EnumerateVersions now only returns versions for the exact object key provided in the request.
- Made Metrics util module public.

## [0.3.1] - 2025-10-22

### Added
- OMPE-69214: Updated storage-core-builder to 0.3.1
- OMPE-69214: Updated storage-core-util to 0.3.1
- OMPE-69214: Updated storage-core-protos to 0.24.0

### Fixed
- Made S3 and Azure modules public.

## [0.3.0] - 2025-10-21

### Added
- OMPE-64246: Added a serve_with_incoming method to the StorageService; integration tests moved to storage-core-integration-tests crate.
- OMPE-67765: Update to storage-protos 0.23.0. This added v1beta versions of all services.
- OMPE-67769: Return Versioned Resource Addresses from EnumerateVersions.

## [0.2.4] - 2025-09-10

### Added
- OMPE-62953: Improved Config and Secrets handling and documentation.
- OMPE-63352: Implement `list_services` in the StorageService builder itself using the dynamic set of services registered with it.
- Updated storage-core-builder to 0.2.4
- Updated storage-core-util to 0.2.4
- Updated storage-core-protos to 0.2.1

### Fixed
- OMPE-61458: regression. do not require content-length in resource ids. some resource ids may only have etags.
- Update crates.

## [0.2.3] - 2025-08-25

### Added
- Updated storage-core-builder to 0.2.3
- Updated storage-core-util to 0.2.3

## [0.2.2] - 2025-08-21

### Added
- Switched to using the `Semaphore` from `storage-core-util` for the egress throttle to enable metrics.
- Updated storage-core-builder to 0.2.2
- Updated storage-core-util to 0.2.2

## [0.2.1] - 2025-08-19

### Added
- Made layer and backend fields public to allow external code to access them if they implement additional traits for them.
- Updated storage-core-builder to 0.2.1
- Updated storage-core-util to 0.2.1
