# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.4.1] - 2026-01-14

### Added
- OMPE-75559: AWS AssumeRole / STS / IRSA support for S3 backend.
- `copy_after_source_change` and `write_between_stat_and_read` exercise the etag-identity conflict path on non-versioned buckets across all backends; `enumerate_versions_never_short_circuits_by_default` pins that versioning is never auto-detected (`FAILED_PRECONDITION` only ever comes from an explicit `versioning_enabled = false` config).

## [0.3.4] - 2025-12-03

### Fixed
- Fixed test involving version ids returned from write, copy, move operations.

## [0.3.3] - 2025-12-01

### Added
- OMPE-64175: List cache layer. Replaces stat cache layer, caching results for stat(), list(), and list_stat().
- Updated storage-core to 0.3.3
- Updated storage-core-test to 0.3.3
- Updated storage-core-util to 0.3.2

### Fixed
- Fixed chunking logic in the write_data_multipart function to make the correct number of chunks.

## [0.3.2] - 2025-11-04

### Added
- OMPE-70304: Updated storage-core to 0.3.2

## [0.3.1] - 2025-10-24

### Added
- OMPE-69214: Updated storage-core to 0.3.1

## [0.3.0]

### Added
- OMPE-67765: Update to storage-protos 0.23.0. This added v1beta versions of all services. Most tests now are disabled by default. Use '--features full' to run all tests.

## [0.1.0]

### Added
- OMPE-64246: Initial Release
