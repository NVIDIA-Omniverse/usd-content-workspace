# Storage Core Protos

Protocol buffer definitions for the Storage Core service APIs. This crate contains the gRPC service definitions and message types used by storage-core applications.

## Current API Version

**Version**: 0.28.0 + the native-metadata v1alpha hunks from storage-protos 1.0.0-beta.6 and the `IsReadOnly` / `IsVersioned` RPCs, all applied by hand — see proto/storage-protos/VERSION  
**Repository**: the `storage/api` folder of this repository
**Hash**: df15f41445871c218a79b8af64d746b3b30378a2

> **Local patch — read before re-vendoring.** `FileObjectService.IsReadOnly` and
> `VersioningService.IsVersioned` (both v1alpha) are present in the
> vendored tree but **not** in tag 0.28.0; they were added ahead of the upstream
> release. Upstream released them in storage-protos tag 1.0.0-beta.6, which
> this crate has not re-vendored to. Copying the tree at 0.28.0 will
> silently delete both RPCs and break every consumer in storage-core. See
> `proto/storage-protos/VERSION` for the full note and the re-vendor checklist.

## Included Services

### FileObject Service (v1alpha)
The core file object operations service providing:

- **Stat**: Get metadata and resource identity for a file
- **Read**: Read file data using resource identity
- **ReadFromAddress**: Read file data using resource address
- **Write**: Write file data with support for streaming, redirects, and multipart uploads
- **Delete**: Delete files from storage
- **Enumerate**: List files in a directory
- **Copy**: Copy files within storage
- **Move**: Move files within storage
- **GetOptimisticLockingSupport**: Query optimistic-locking and create-only support
- **IsReadOnly**: Report whether an address is read-only *(local patch, not in 0.28.0)*
- **FetchWriteTypeInfo**: Get upload method recommendations based on file size
- **CompleteRedirectUpload**: Complete uploads performed via redirect URLs
- **UploadPart**: Get presigned URLs for multipart upload parts
- **CompleteMultipartUpload**: Complete multipart uploads
- **AbortMultipartUpload**: Abort in-progress multipart uploads

### FileFolder Service (v1alpha)
Folder and directory operations:

- **List**: List contents of a folder (streaming response)
- **ListStat**: List folder contents with metadata (streaming response)
- **DeleteFolder**: Delete empty folders

### Capabilities Service (v1alpha)
Service discovery and capability information:

- **ListServices**: Get list of implemented services and versions
- **ListTopLevelAddresses**: Get top-level resource addresses for browsing
- **ListRoutes**: Get routing patterns for resource addresses

### Versioning Service (v1alpha)
File versioning operations:

- **EnumerateVersions**: List versions of a file (streaming response)
- **IsVersioned**: Report whether an address has versioning enabled *(local patch, not in 0.28.0)*

### Metadata Service (v1alpha)
Metadata operations:

- **GetMetadata**: Read metadata keys for a URI
- **UpdateMetadata**: Update one metadata key
- **UpdateMetadataBatch**: Update multiple metadata keys with per-entry results
- **DeleteMetadata**: Delete one metadata key


### Generated Code Structure

```rust
pub mod nvidia {
    pub mod omniverse {
        pub mod storage {
            pub mod fileobject {
                pub mod v1alpha { /* FileObject service */ }
                pub mod v1beta { /* FileObject service */ }
            }
            pub mod filefolder {
                pub mod v1alpha { /* FileFolder service */ }
                pub mod v1beta { /* FileFolder service */ }
            }
            pub mod capabilities {
                pub mod v1alpha { /* Capabilities service */ }
                pub mod v1beta { /* Capabilities service */ }
            }
            pub mod versioning {
                pub mod v1alpha { /* Versioning service */ }
                pub mod v1beta { /* Versioning service */ }
            }
            pub mod metadata {
                pub mod v1alpha { /* Metadata service */ }
            }
        }
    }
}
```

## Dependencies

- **prost**: Protocol buffer serialization
- **tonic**: gRPC framework
- **serde**: JSON serialization for REST APIs
- **utoipa**: OpenAPI documentation generation
- **time**: Timestamp handling

## License

This project is licensed under the Apache License, Version 2.0. See [LICENSE](../LICENSE).
