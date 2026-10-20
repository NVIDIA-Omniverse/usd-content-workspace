# Local process deployment (no Kubernetes)

> **Applies to:** Discovery, Storage, and Metadata run as ordinary processes on one machine · **Released:** no released artifact targets this mode; NGC ships charts and images for Kubernetes · **This repo's snapshot:** Auto Discovery operator 1.2.0 (static-endpoint mode), Storage Service 1.2.3 + unreleased, Storage Core with unreleased changes (source commit in [`SOURCE_VERSIONS.md`](../../../../SOURCE_VERSIONS.md)), Python filesystem example in unreleased Storage API source (after 1.0.0-beta.4) · **Sources:** operator [`README.md`](../../../../discovery/reference/discovery-service/operator/README.md) "Running Locally", [`config.rs`](../../../../discovery/reference/discovery-service/operator/src/config.rs), [`static_discovery.rs`](../../../../discovery/reference/discovery-service/operator/src/static_discovery.rs); Storage Service [`args.rs`](../../../../storage/reference/storage-service/storage-service/src/args.rs); Storage Core [`docs/`](../../../../storage/reference/storage-core/docs/); [`filesystem_example/README.md`](../../../../storage/api/filesystem_example/README.md)

This is an optional third way to run the stack, beside a local Kubernetes cluster (MicroK8s, kind; see [local-development.md](local-development.md)) and a production cluster ([production-stack.md](production-stack.md)). Use it for a single developer or a workstation-local content service: no cluster, no Helm, no ingress. Clients still connect the normal way, through a Discovery URL and the canonical Storage and Metadata APIs, so anything validated here also works against a cluster deployment.

It also shows that the services in this repository are **reference implementations**: a local process that serves the same contracts can stand in for all of them ([../stack/replaceable-and-modular.md](../stack/replaceable-and-modular.md)).

## What a local deployment must provide

However it is built, a local process deployment is correct when:

1. **Discovery** answers `GET /api/v2/services` with `schema-version: 2`, listing only services that are running, with URLs a client can reach (never `0.0.0.0`). Serve `/api/v1/services` (`schema-version: 1`) as well for clients that read only v1; Navigator 1.0.4 falls back to v1.
2. **Storage** serves the Storage API wire versions clients need — `nvidia.omniverse.storage.{fileobject,filefolder,versioning,capabilities}.v1alpha` and `.v1beta`, over gRPC and REST — from the definitions in [`storage/api`](../../../../storage/api/). Copy, Move, CreateFolder, and ListRoutes exist only in `v1alpha`; do not invent `v1beta` versions of them.
3. **Metadata** serves `nvidia.omniverse.storage.metadata.v1alpha.MetadataService` and, for current clients, `nvidia.omniverse.metadata.v1beta.MetadataService` ([`metadata/api`](../../../../metadata/api/)). There is no `storage.metadata.v1beta`.
4. It passes the Storage API conformance suite for every operation it claims ([../test/conformance.md](../test/conformance.md)).

Record the API versions the processes implement in the version record ([../versions/README.md](../versions/README.md)); there is no chart version to read.

## Three ways to build it

| Way | Effort | Fits | Read |
| --- | --- | --- | --- |
| Run this repo's reference services as processes | Low | Trying the APIs; developing against the real Storage Service | [local-process/reference-services.md](local-process/reference-services.md) |
| Build a single local host from this repo's libraries and contracts | High | A durable workstation content service with local disk, versions, and metadata in one executable | [local-process/single-host.md](local-process/single-host.md) |
| Put a static Discovery catalog in front of any storage process | Lowest | A storage process that cannot answer the Discovery capabilities call | [local-process/reference-services.md#static-catalog](local-process/reference-services.md#static-catalog) |

Validation for all three: [local-process/validate.md](local-process/validate.md).

## Connect clients

```bash
curl -fsS http://127.0.0.1:<discovery-port>/api/v2/services | jq .
export OMNI_STORAGE_DISCOVERY=http://127.0.0.1:<discovery-port>   # before launching Kit
```

Navigator takes the same Discovery URL ([navigator.md](navigator.md)). For another machine to connect, give it the host's real address, never the wildcard listen address.

## Security

- **Keep it on loopback** unless other machines must connect. Local processes in this mode usually run without authentication, so any client that can reach the port can read and change content.
- Not every process can bind loopback. The Storage Service can: set `rest.addr`, `grpc.addr`, and `grpc_probe_service.addr` to `127.0.0.1` (all default to `0.0.0.0`). The Auto Discovery operator always binds `0.0.0.0` on both ports ([`main.rs`](../../../../discovery/reference/discovery-service/operator/src/main.rs)), and the Python filesystem example binds every interface for REST and gRPC. Keep those off the LAN with a host firewall, or run them in a container that publishes ports only on `127.0.0.1`. Advertise loopback URLs in Discovery.
- Binding to a LAN interface does not add TLS. To share beyond one workstation, put the processes behind a TLS-terminating proxy that validates tokens, or move to a cluster deployment with the Envoy auth extension ([../stack/security.md](../stack/security.md)).
- Do not advertise an authentication provider that is not configured; clients will try to sign in against it.
- Keep backend credentials in the platform's credential store or environment, never in shared config files or in URLs.
- Back up metadata consistently: stop the process, or use the database's own backup tool. Copying only the main file of a running SQLite WAL database is not a consistent backup.

## Moving to a cluster

The contracts do not change, so clients do not change: point `OMNI_STORAGE_DISCOVERY` at the cluster's Discovery URL. Content and metadata do not move by themselves; copy content through the Storage API and re-create metadata through the Metadata API. Then grow the deployment as needed ([../stack/choosing.md](../stack/choosing.md)).
