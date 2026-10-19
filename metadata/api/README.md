# Metadata API

The Metadata API reads and writes key/value metadata on storage resources. It is published as a protocol definition and an OpenAPI specification; implementations and clients generate their own bindings from them.

| Path | Contents |
|------|----------|
| `proto/nvidia/omniverse/metadata/v1beta/metadata.proto` | gRPC service and messages (package `nvidia.omniverse.metadata.v1beta`) |
| `openapi/metadata/v1beta/metadata-api.yaml` | REST API, bundled into a single OpenAPI document |
| `CHANGELOG.md` | Release notes |
| `LICENSE.txt`, `LICENSE_HEADER.txt` | Apache License 2.0 and the header used in source files |
| `SECURITY.md` | How to report a security issue |

## Relationship to the Storage API

The schema originated in the metadata API of the Storage API (`storage/api` in this repository, package `nvidia.omniverse.storage.metadata.v1alpha`) and stays wire-compatible with it. This API is the top-level `nvidia.omniverse.metadata.*` contract; the intentional differences are listed in [CHANGELOG.md](CHANGELOG.md).

## Generating bindings

Use any protobuf toolchain, for example:

```bash
protoc -I proto --python_out=. --grpc_python_out=. proto/nvidia/omniverse/metadata/v1beta/metadata.proto
```

For REST, feed `openapi/metadata/v1beta/metadata-api.yaml` to an OpenAPI generator of your choice.

## License

Apache License 2.0. See [LICENSE.txt](LICENSE.txt).
