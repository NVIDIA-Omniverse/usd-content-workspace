# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.9.0-alpha.1]

### Added

- gRPC and REST test clients for the `IsReadOnly` and `IsVersioned` capability RPCs (OMPE-104659)


## [0.8.0]


### Changed

- Update packages


### Fixed

- OMPE-104009 canonicalize resource identity URL paths


## [0.7.3]


### Changed

- Updated the following local packages: storage-core-util, storage-core-builder, storage-core


## [0.7.2]


### Added

- OMPE-98507 FAILED_PRECONDITION for EnumerateVersions when versioning is configured off


## [0.7.1]


### Changed

- Updated the following local packages: storage-core-util, storage-core, storage-core-builder


## [0.7.0]


### Added

- OMPE-96515 workspace manifest overlay (.wkspc/.manifest)


### Changed

- Update LICENSE to Apache 2.0 and copyrights in preparation for github


## [0.6.2]


### Changed

- Updated the following local packages: storage-core-util, storage-core, storage-core-builder


## [0.6.1]


### Changed

- Updated the following local packages: storage-core


## [0.6.0]


### Added

- OMPE-94177 update storage protos to 0.28.0
- OMPE-89612 Consistent Resource IDs


## [0.5.13]


### Added

- OMPE-84310 implement second layer cache using redis/valkey
- OMPE-84456 Add metadata api trait, implementations, and cache


## [0.5.12]


### Changed

- Updated the following local packages: storage-core


## [0.5.11]


### Changed

- Update crates


## [0.5.10]


### Added

- OMPE-50888 Logging improvements based on Omniverse Logging SRD


## [0.5.9]


### Changed

- Updated the following local packages: storage-core


## [0.5.8]


### Changed

- Add update rust crates skill


## [0.5.7]


### Changed

- Updated the following local packages: storage-core


## [0.5.6]


### Changed

- Updated the following local packages: storage-core


## [0.5.5]


### Changed

- Update crates. update dev/debugging build profiles to try and reduce build/link times


## [0.5.4]


### Fixed

- Add missing serde feature to Url crate dependency


## [0.5.3]


### Added

- OMPE-79307 add cloudfront support to s3 backend.


### Fixed

- OMPE-80756 mask sensitive gRPC headers in debug output.


## [0.5.2]


### Changed

- Updated the following local packages: storage-core-util, storage-core


## [0.5.1]


### Changed

- Updated the following local packages: storage-core-util, storage-core


## [0.5.0]


### Added

- OMPE-78424: Change observability stack to OpenTelemetry


## [0.4.2]


### Changed

- Updated the following local packages: storage-core-builder, storage-core-util, storage-core


## [0.4.1] - 2026-01-14

### Added
- OMPE-75559: AWS AssumeRole / STS / IRSA support for S3 backend.

## [0.3.3] - 2025-12-01

### Added
- OMPE-64175: List cache layer. Replaces stat cache layer, caching results for stat(), list(), and list_stat().
- Updated storage-core to 0.3.3
- Updated storage-core-builder to 0.3.2
- Updated storage-core-util to 0.3.2

## [0.3.2] - 2025-11-04

### Added
- OMPE-70304: Updated storage-core to 0.3.2

## [0.3.1] - 2025-10-24

### Added
- OMPE-69214: Updated storage-core to 0.3.1
- OMPE-69214: Updated storage-core-builder to 0.3.1
- OMPE-69214: Updated storage-core-util to 0.3.1

## [0.3.0] - 2025-10-21

### Added
- OMPE-64246: Added a new `StorageServiceRunner` helper to launch a storage service in a new thread and provide clients attached to it.
- OMPE-64246: Now uses the exported types from `storage-core` instead of the types directly from `storage-core-protos`.
- OMPE-67765: Update to storage-protos 0.23.0. This added v1beta versions of all services.

## [0.2.4] - 2025-09-10

### Added
- OMPE-62594: Initial Release. Refactored from storage-core to be a standalone crate.
- Updated storage-core-protos to 0.2.1
