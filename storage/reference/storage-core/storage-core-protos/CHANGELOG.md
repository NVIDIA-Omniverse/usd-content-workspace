# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.29.0-alpha.1]

### Added

- OMPE-104659 `FileObject.IsReadOnly` (v1alpha) and `Versioning.IsVersioned` (v1alpha) capability RPCs
- OMPE-90246 native metadata fields on the `FileObject` stat, read and write messages

### Changed

- The vendored `storage-protos` tree carries these RPCs ahead of the upstream release; `proto/storage-protos/VERSION` records the local patch and the re-vendor checklist


## [0.28.2]


### Changed

- Update packages


## [0.28.1]


### Changed

- Update LICENSE to Apache 2.0 and copyrights in preparation for github


## [0.28.0]


### Added

- OMPE-94177 update storage protos to 0.28.0


## [0.25.6]


### Added

- OMPE-84310 implement second layer cache using redis/valkey
- OMPE-84456 Add metadata api trait, implementations, and cache


## [0.25.5]


### Added

- OMPE-30119 Add S3 rate-limit metrics via SDK interceptor


## [0.25.4]


### Changed

- Update crates. update dev/debugging build profiles to try and reduce build/link times


## [0.25.3]


### Added

- OMPE-79307 add cloudfront support to s3 backend.


## [0.25.2]


### Changed

- Update all dependencies to the latest versions


## [0.25.1]


### Added

- OMPE-69308 Add Manifest Layer


## [0.25.0]

### Added

- OMPE-69996: Update to storage-protos 0.25.0. This comes with fixes to the conformance tests.

## [0.24.0] - 2025-10-24

### Added

- OMPE-69214: Update to storage-protos 0.24.0. This changed the string representation of the FolderMode enum in the GetFolderModeResponse.

## [0.23.0] - 2025-10-21

### Added

- OMPE-67765: Update to storage-protos 0.23.0

## [0.2.1] - 2025-09-02

### Fixed

- Update crates.
