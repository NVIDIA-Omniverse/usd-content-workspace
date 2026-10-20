# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.11.0-alpha.1]

### Security

- Cap the Azure SDK log targets so `azure_core` 0.21 cannot write the `Authorization` header into the logs (RUSTSEC-2026-0275, compensating control). The cap is a filter layer ANDed with the `EnvFilter` rather than a directive, because `EnvFilter` evaluates span directives first and resolves the most specific target directive, either of which would otherwise re-enable the credential log. It applies to the OTLP exporter as well as stdout, and has no opt-out
- The cap is only applied when the global level is more verbose than `info`; `EnvFilter` directives are absolute, so adding it under `RUST_LOG=warn` would have raised these targets rather than capped them


### Added

- `Error::Unimplemented::with_message` to describe what is unimplemented in the gRPC status and `Display`


## [0.10.0]


### Changed

- Update packages


### Fixed

- OMPE-104009 canonicalize resource identity URL paths


## [0.9.0]


### Changed

- **small-object-cache:** Replace replay coalescing with moka single-flight


## [0.8.1]


### Fixed

- OMPE-92921 block access to unconfigured S3/Azure endpoints


## [0.8.0]


### Added

- OMPE-96515 workspace manifest overlay (.wkspc/.manifest)


### Changed

- Update LICENSE to Apache 2.0 and copyrights in preparation for github


### Fixed

- Pool Redis L2 connections and raise default command timeout


## [0.7.1]


### Fixed

- OMPE-95382 replace mpsc-channel streaming with lazy async_stream BoxStreams to bound memory


## [0.7.0]


### Added

- OMPE-94177 update storage protos to 0.28.0


### Fixed

- Fix OMPE-93491 The L2 cache changes can serve stale data after prefix invalidations and can leave local stale entries


## [0.6.0]


### Added

- OMPE-84310 implement second layer cache using redis/valkey
- OMPE-92431 cancel tasks
- OMPE-92558 Add configurable metric prefix
- OMPE-84456 Add metadata api trait, implementations, and cache


### Changed

- OMPE-78424 Update Otel service.namespace to omniverse


## [0.5.8]


### Changed

- Update crates
- Clean up misleading and unhelpful log messages


## [0.5.7]


### Added

- OMPE-87307 add structured error details for gRPC and REST
- OMPE-88199 secrets hot-reload via file polling
- OMPE-69428 Add env var-based HTTP headers to AdditionalHeaders layer
- OMPE-30119 Add S3 rate-limit metrics via SDK interceptor
- OMPE-50888 Logging improvements based on Omniverse Logging SRD


### Changed

- Suppress repeated log spam when secrets file is missing
- Migrate all tracing macros to rate-limited variants


## [0.5.6]


### Changed

- Add update rust crates skill


## [0.5.5]


### Changed

- Update crates. update dev/debugging build profiles to try and reduce build/link times


## [0.5.4]


### Changed

- Updated the following local packages: storage-core-builder


## [0.5.3]


### Added

- OMPE-79307 add cloudfront support to s3 backend.
- OMPE-80825 Allow using Azure FrontDoor for reads


### Fixed

- OMPE-80756 mask sensitive gRPC headers in debug output.


## [0.5.2]


### Changed

- Handle OTEL exporter env vars being 'none'


## [0.5.1]


### Changed

- Improve log formatting


## [0.5.0]


### Added

- OMPE-78424: Change observability stack to OpenTelemetry


### Changed

- Update all dependencies to the latest versions


## [0.4.3]


### Added

- OMPE-77492 Add source_location to all errors


### Fixed

- OMPE-77659 Fix size of cache_errors array


## [0.4.2] - 2026-01-14

### Added
- OMPE-75559: AWS AssumeRole / STS / IRSA support for S3 backend.

## [0.4.1] - 2025-12-11

### Fixed
- OMPE-74954: Fix parsing of timeouts.operation_attempt, make timeouts.keepalive optional. Add tests for serde serialize and deserialize.

## [0.3.2] - 2025-12-01

### Added
- OMPE-64175: List cache layer. Replaces stat cache layer, caching results for stat(), list(), and list_stat().
- Updated storage-core-builder to 0.3.2
- Updated storage-core-protos to 0.25.0

## [0.3.1] - 2025-10-24

### Added
- OMPE-69214: Updated storage-core-builder to 0.3.1
- OMPE-69214: Updated storage-core-protos to 0.24.0

## [0.3.0] - 2025-10-21

### Added
- OMPE-64246: Added a serve_with_incoming method to the MetricsServer.
- OMPE-67765: Update to storage-protos 0.23.0. This added v1beta versions of all services.

## [0.2.4] - 2025-09-10

### Added
- OMPE-62953: Improved Config and Secrets handling and documentation.
- Updated storage-core-builder to 0.2.4

### Fixed
- Update crates.

## [0.2.3] - 2025-08-25

### Added
- Updated storage-core-builder to 0.2.3

## [0.2.2] - 2025-08-21

### Added
- Added `Semaphore` to the library. This is a wrapper around the tokio semaphore that provides metrics for the semaphore.
- Updated storage-core-builder to 0.2.2

## [0.2.1] - 2025-08-19

### Added
- Updated storage-core-builder to 0.2.1
