# Run the reference services as local processes

> **Applies to:** Auto Discovery operator, Storage Service, Python filesystem example · **Released:** not a released mode · **This repo's snapshot:** operator 1.2.0, Storage Service 1.2.3 + unreleased, unreleased Storage API source (after 1.0.0-beta.4) · **Sources:** operator [`README.md`](../../../../../discovery/reference/discovery-service/operator/README.md), [`config.rs`](../../../../../discovery/reference/discovery-service/operator/src/config.rs), [`static_discovery.rs`](../../../../../discovery/reference/discovery-service/operator/src/static_discovery.rs); Storage Service [`args.rs`](../../../../../storage/reference/storage-service/storage-service/src/args.rs), [`README.md`](../../../../../storage/reference/storage-service/README.md); [`filesystem_example/README.md`](../../../../../storage/api/filesystem_example/README.md); [Discovery Catalog API](../../../../../discovery/reference/discovery-service/operator/docs/api/docs/discovery-catalog-api.md)

Back to [local process deployment](../local-process.md).

## Discovery: the operator in static-endpoint mode

The Auto Discovery operator can skip Kubernetes and poll fixed endpoints. Build it in [`discovery/reference/discovery-service/operator`](../../../../../discovery/reference/discovery-service/operator/) with `cargo build --release --locked`, then:

```bash
./target/release/auto-discovery-service \
  --static-endpoint storage=http://127.0.0.1:<storage-grpc-port>#http://127.0.0.1:<storage-rest-port>
```

- Format: `NAME=GRPC_URL` or `NAME=GRPC_URL#REST_URL`. Repeat the flag, or set `STATIC_ENDPOINTS` to a comma-separated list.
- The operator calls `nvidia.omniverse.discovery.capabilities.v2beta.CapabilitiesService/ListServices` on each gRPC URL, retries on the deprecated `v2alpha` method when the target answers `UNIMPLEMENTED`, and re-queries periodically.
- It serves the internal API on 8080 and the external API on 8081 by default: `curl http://127.0.0.1:8081/api/v2beta/services` (`/api/v2/services` is an alias with the same body).

The target must implement the **Discovery** capabilities service, v2beta or v2alpha. The Storage Service implements v2alpha (`storage-service/src/api/discovery/`), which the operator accepts with a deprecation warning. The Python filesystem example serves only the **Storage** capabilities services (`storage.capabilities.v1alpha` and `v1beta`), so the operator cannot query it and leaves it out of the catalog. Put a [static catalog](#static-catalog) in front of it instead, or add the Discovery capabilities service to it (v2beta for new code).

The operator serves `/api/v1/services` (and `/api/v1alpha/services`) as well as `/api/v2beta/services` and `/api/v2/services`, for clients that read only the v1 catalog.

## Storage: the Python filesystem example

The simplest local storage process. It serves REST on 8011 and gRPC on 50051 by default (`--http-port`, `--grpc-port`), addresses content under the base URI `file-storage://fileservice` by default (`FILESERVICE_SERVER_BASE_URI`), and keeps files, versions, and generic metadata on local disk. Generate the Python protobuf stubs, then run it with Poetry as described in [`filesystem_example/README.md`](../../../../../storage/api/filesystem_example/README.md). Use it for learning and adapter development, not for shared or durable content.

## Storage and Metadata: the Storage Service as a process

The Rust Storage Service reads its whole configuration from TOML or JSON: `--config-file <path>` (or `STORAGE_SERVICE_CONFIG_FILE`), `STORAGE_SERVICE_CONFIG_TOML`, or `STORAGE_SERVICE_CONFIG_JSON`. In Kubernetes the chart renders that TOML into a ConfigMap and passes it through `STORAGE_SERVICE_CONFIG_TOML`. Outside a cluster:

- It needs an S3-compatible or Azure Blob backend; it has no local-filesystem backend. A local S3-compatible server works for development.
- A metadata database (for example PostgreSQL) is optional: the `metadata` section of the configuration is optional, and without it the Metadata APIs are not served. Add one when clients need metadata.
- Its listeners default to `0.0.0.0`; set `rest.addr`, `grpc.addr`, and `grpc_probe_service.addr` to `127.0.0.1` for a loopback-only process.
- No standalone TOML reference is published; [`docs/helm/`](../../../../../storage/reference/storage-service/docs/helm/) documents settings by Helm key. Render the chart for your values, copy the generated `storage-service` ConfigMap TOML, and replace in-cluster hostnames and Secret-mounted paths with local ones.
- Run once with `--bootstrap-only` to apply metadata migrations, then start it normally.
- It serves both metadata APIs (`storage.metadata.v1alpha` and `metadata.v1beta`) from its metadata backend, so Storage Service plus the operator gives Discovery, Storage, and Metadata.
- The Storage Service README refers to `scripts/local-bootstrap.sh` and `local-teardown.sh` for its test dependencies; they are not in this snapshot.

## Static catalog

When the storage process does not implement the Discovery capabilities service, serve a fixed v2 catalog (and a v1 catalog at `/api/v1/services` for clients that read only v1; see the operator's [v1 response schema](../../../../../discovery/reference/discovery-service/operator/docs/v1-response-schema.md)) that follows the [Discovery Catalog API](../../../../../discovery/reference/discovery-service/operator/docs/api/docs/discovery-catalog-api.md) from any static web server at `/api/v2/services`, the path Storage Navigator reads (serve the same file at `/api/v2beta/services` to mirror the snapshot operator). List each REST resource with its version keys and paths, and each fully qualified gRPC service name the process actually serves, for example `nvidia.omniverse.storage.fileobject.v1beta.FileObjectService`. A static catalog does not notice when a service stops; keep it in step by hand.
