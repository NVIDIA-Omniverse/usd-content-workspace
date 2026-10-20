---
name: local-process-stack
description: >-
  Run or build usd-content-workspace Discovery, Storage, and Metadata as local processes on one
  machine, with no Kubernetes or MicroK8s. Use for requests such as "run the storage stack as a local
  process", "I need discovery and storage on my workstation without a cluster", or "build a single
  local executable for discovery, storage and metadata". Covers running this repo's reference services
  as processes and building a single host from Storage Core and the API contracts, plus validation.
  Not for cluster deployments.
license: Apache-2.0
---

# Local process stack

Deliver a working local deployment and verified client connection instructions, not only a design. The detailed references live in the expert skill; this skill is the task procedure.

## Choose the path

Ask what the caller needs if it is not clear, then pick one:

| Need | Path | Reference |
| --- | --- | --- |
| Try the APIs or develop against the real Storage Service | Run the reference services as processes: Discovery operator in static-endpoint mode plus the Storage Service or the Python filesystem example | [reference-services.md](../usd-content-workspace-expert/references/deploy/local-process/reference-services.md) |
| A durable workstation content service on local disk, with versions and metadata, in one executable | Build a single host on Storage Core and the contracts | [single-host.md](../usd-content-workspace-expert/references/deploy/local-process/single-host.md) |
| A storage process that cannot answer the Discovery capabilities call | Add a static catalog | [reference-services.md#static-catalog](../usd-content-workspace-expert/references/deploy/local-process/reference-services.md#static-catalog) |

Requirements every path must meet: [local-process.md](../usd-content-workspace-expert/references/deploy/local-process.md).

## Procedure

1. **Inspect first.** Read the overview and the chosen path's reference, then the component files it links (`README.md`, `AGENTS.md`, protos, `docs/`). Record the repo commit and the API versions you will serve.
2. **Preserve state.** For an existing local deployment, keep its configuration, mounts, metadata database, journals, and any identity or signing keys. For a new one, use a new named directory; never initialize a managed store in a non-empty ordinary folder. Ask for the managed storage directory before creating it, and use a disposable directory for building and testing meanwhile.
3. **Default safely.** Keep everything on loopback: bind the Storage Service to `127.0.0.1`, and keep the Discovery operator and filesystem example (which always listen on every interface) behind a host firewall or a container that publishes ports only on loopback. Ask before exposing anything to the LAN, and say plainly that binding to a LAN interface does not add TLS or authentication.
4. **Build in working stages** (single-host path): Discovery and bindings, managed filesystem storage, metadata, optional S3, optional UI. Keep the contract handlers, backend interface, and adapters separate. Live Edit is optional and must not block Discovery, Storage, and Metadata.
5. **New code stays out of snapshots.** Component directories here are released snapshots owned upstream. Put a new host in its own directory or repository and follow [`CONTRIBUTING.md`](../../CONTRIBUTING.md).
6. **Validate with disposable data**, following [validate.md](../usd-content-workspace-expert/references/deploy/local-process/validate.md): Discovery shapes and reachability, Storage API conformance, storage behavior beyond the suite, metadata through both services, and a client check through Kit or Navigator.
7. **Then start the real instance** with its preserved state and run read-only checks. Do not kill processes you did not start or cannot identify, overwrite saved data, or claim a running process uses a new binary without checking.

## Finish with evidence

Report the build commit and executable (or the commands for each reference process), the exact run commands, the Discovery URL and the HTTP and gRPC addresses (loopback and any LAN address, HTTP versus HTTPS), the state and log locations, and the validation results, separating observed results from untested platforms and S3 fixtures from real cloud tests. Do not claim a supervisor or start-at-login setup unless you configured and verified one.

For anything beyond this procedure — versions, other deployment targets, security, extending the stack — use the [usd-content-workspace expert](../usd-content-workspace-expert/SKILL.md).
