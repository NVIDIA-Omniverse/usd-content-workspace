# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.8.0-alpha.2]

### Changed

- File-object writes truncate bodies to `data_object_size` and reject shorter bodies, preventing resource exhaustion from oversized requests.
- The `IsVersioned` handlers pass the whole request, extensions included, to `Versioning::is_versioned`


## [0.8.0-alpha.1]

### Added

- OMPE-104659 route and serve the `IsReadOnly` and `IsVersioned` capability RPCs over gRPC and REST
- `RestIngress::without_swagger_ui()` to stop serving the API documentation


### Changed

- **Breaking: `ServiceInfoProvider::service_info` returns `Option<ServiceInfo>` and `list_in_services` is removed.** Implementors that are not part of the storage capability surface return `None`; the previous contract forced every service to supply a `ServiceInfo` it had no sane default for
- The `IsVersioned` handlers forward the caller's request metadata, so the manifest layer's cold-cache read carries the caller's credentials instead of being issued bare


## [0.7.0]


### Changed

- Update packages


### Fixed

- OMPE-104009 canonicalize resource identity URL paths


## [0.6.5]


### Added

- Let services opt out of the storage capabilities ListServices


## [0.6.4]


### Added

- OMPE-98507 FAILED_PRECONDITION for EnumerateVersions when versioning is configured off


## [0.6.3]


### Changed

- Updated the following local packages: storage-core-util


## [0.6.2]


### Added

- `Versioning::is_versioning_enabled(top_level_address) -> Result<bool>` trait method (v1alpha and v1beta) exposing a versioning-capability probe to callers such as storage-service.


### Changed

- Update LICENSE to Apache 2.0 and copyrights in preparation for github


## [0.6.1]


### Changed

- Updated the following local packages: storage-core-util


## [0.6.0]


### Added

- OMPE-94177 update storage protos to 0.28.0
- Add production boxed builders and shared metrics state


## [0.5.7]


### Added

- OMPE-84310 implement second layer cache using redis/valkey
- OMPE-84456 Add metadata api trait, implementations, and cache


### Changed

- Fix ServiceBuilder documentation about layer ordering


## [0.5.6]


### Changed

- Update crates
- Clean up misleading and unhelpful log messages


## [0.5.5]


### Added

- OMPE-87307 add structured error details for gRPC and REST
- OMPE-50888 Logging improvements based on Omniverse Logging SRD


### Changed

- Migrate all tracing macros to rate-limited variants


## [0.5.4]


### Changed

- Add update rust crates skill


## [0.5.3]


### Changed

- Update crates. update dev/debugging build profiles to try and reduce build/link times


## [0.5.2]


### Fixed

- Add missing serde feature to Url crate dependency


## [0.5.1]


### Added

- OMPE-79307 add cloudfront support to s3 backend.


### Fixed

- OMPE-80756 mask sensitive gRPC headers in debug output.


## [0.5.0]


### Added

- OMPE-78424: Change observability stack to OpenTelemetry


### Changed

- Convert cargo test to nextest, shard over multiple gitlab runners


## [0.4.2]


### Added

- OMPE-77492 Add source_location to all errors


## [0.4.1] - 2026-01-14

### Added
- OMPE-75559: AWS AssumeRole / STS / IRSA support for S3 backend.

## [0.3.2] - 2025-12-01

### Added
- OMPE-64175: List cache layer. Replaces stat cache layer, caching results for stat(), list(), and list_stat().
- Updated storage-core-protos to 0.25.0

### Fixed
- OMPE-69996: Fixed the CreateFolder REST function to return a 409 error when a file already exists at the folder address.

## [0.3.1] - 2025-10-24

### Added
- OMPE-69214: Updated storage-core-protos to 0.24.0

## [0.3.0] - 2025-10-21

### Added
- OMPE-64246: Added a serve_with_incoming method to GrpcIngress and RestIngress.
- OMPE-67765: Update to storage-protos 0.23.0. This added v1beta versions of all services.

## [0.2.4] - 2025-09-10

### Added

- OMPE-62594: Refactored test code to use the new storage-core-test crate.
- OMPE-63352: Implement `list_services` in the StorageService builder itself using the dynamic set of services registered with it.
- Updated storage-core-protos to 0.2.1

### Fixed

- Update crates.

## [0.2.3] - 2025-08-25

### Fixed
- OMPE-60861: Made logging consistent in all GRPC and REST ingress functions to log the error code, the context, and the error message.


## [0.2.2] - 2025-08-21

### Fixed
- Fixed all GRPC functions that return a stream to ensure that all traces in the stream occur in the correct span.

## [0.2.1] - 2025-08-19

### Fixed
- Added health check to grpc server.
