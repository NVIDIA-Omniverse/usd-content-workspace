# Permission Service

## Start here

Read the [service overview and architecture](docs/service/docs/index.md) for the service's purpose, use cases, authorization request flow, and integrations. The overview links to the deployment, authentication, policy, and operational documentation.

## Development

### gRPC

The Rust code for the gRPC APIs is generated from protobuf definitions and checked into the `src` directory of each
proto crate, so building and testing need no protobuf tooling.

| Crate | Proto input |
|---|---|
| `permission-service-proto` | `protos` directory in this repository |
| `permission-service-capabilities-proto` | Omniverse Discovery Service API |
| `permission-service-notification-proto` | Omniverse Notification Service API |

## Run

The service supports two database backends controlled by the `DATABASE_TYPE` environment variable:

- **`postgres`** (default) — uses PostgreSQL as a read-write storage backend.
- **`config-file`** — uses local YAML configuration files as a read-only storage backend.

See [Database Configuration](docs/service/docs/database-configuration.md) for the full reference on environment variables and file formats.

### Run with Docker (PostgreSQL)

1. Build the image:
   ```shell
   docker build -t permission-service -f docker/Dockerfile .
   ```

2. Create a network and a volume for Postgres:
   ```shell
   docker network create permission-network
   docker volume create postgres-volume
   ```

3. Run Postgres:
   ```shell
   docker run --name postgres --mount "type=volume,src=postgres-volume,target=/var/lib/postgresql/data" --network permission-network -p 5432:5432 -e POSTGRES_USER=postgres -e POSTGRES_PASSWORD=postgres -d postgres:17.2-alpine
   ```

4. Run the service:
   ```shell
   docker run --name permission-service --network permission-network -p 3000:3000 -e ENABLE_AUTH=false -e BUNYAN=false -e POSTGRES_HOST=postgres -e POSTGRES_USER=postgres -e POSTGRES_PASSWORD=postgres -e POSTGRES_DBNAME=postgres permission-service
   ```

5. Stop containers:
   ```shell
   docker stop postgres permission-service
   ```

### Run with Docker (Config File)

1. Build the image:
   ```shell
   docker build -t permission-service -f docker/Dockerfile .
   ```

2. Create a `policies.yaml` file. This file defines Cedar authorization policies and their filtering constraints. Each entry contains a Cedar policy string and optional `action`, `principal`, and `resource` fields that control when the policy is matched during authorization queries. Omitted fields mean the policy applies globally for that dimension.

   ```yaml
   policies:
     - policy: "permit(principal, action, resource);"
     - policy: 'permit(principal, action == Action::"my-service:read", resource);'
   ```

3. Create a `metadata.yaml` file. This file registers services, their actions, and resource types. The service uses this metadata to validate authorization requests and determine evaluation priorities.

   ```yaml
   services:
     - name: "my-service"
       principal:
         idClaim: "sub"                  # optional, defaults to "sub"
       actions:
         - "read"
         - "write"
       resourceTypes:
         - type: "document"
           evaluationPriority: "forbid"  # optional, "permit" or "forbid"
   ```

   See [Database Configuration](docs/service/docs/database-configuration.md) for the full file format reference with more examples.

4. Run the service, mounting the configuration files into the container:
   ```shell
   docker run --name permission-service -p 3000:3000 \
     -v $(pwd)/policies.yaml:/etc/permission-config/policies.yaml:ro \
     -v $(pwd)/metadata.yaml:/etc/permission-config/metadata.yaml:ro \
     -e ENABLE_AUTH=false \
     -e DATABASE_TYPE=config-file \
     -e INIT_POLICIES_FILE=/etc/permission-config/policies.yaml \
     -e INIT_METADATA_FILE=/etc/permission-config/metadata.yaml \
     permission-service
   ```

   In config-file mode, all write operations (adding, updating, or deleting policies and metadata) return HTTP 501. To update the configuration, edit the mounted files on the host — the service detects file changes automatically without a restart.

## Meta Permissions

When authentication is enabled, the service enforces meta permissions — policies that control who can view and modify the service's own policies and metadata. These are regular Cedar policies matched against actions in the `permissions` service.

| Action               | Required for                                                                 |
|----------------------|------------------------------------------------------------------------------|
| `permissions:view`       | Viewing another user's permissions (authorization requests for a different principal). Users can always view their own permissions. |
| `permissions:edit`       | Adding, updating, and deleting policies.                                     |
| `permissions:meta:read`  | Reading service metadata (services, actions, resource types).                |
| `permissions:meta:write` | Modifying service metadata (services, actions, resource types).              |

When using PostgreSQL, these policies are typically provided via `database.init.policies` (or `INIT_POLICIES_FILE`) so they are loaded as system policies on startup:

```yaml
policies:
  - policy: 'permit(principal, action == Action::"permissions:edit", resource);'
  - policy: 'permit(principal, action == Action::"permissions:view", resource);'
  - policy: 'permit(principal, action == Action::"permissions:meta:read", resource);'
  - policy: 'permit(principal, action == Action::"permissions:meta:write", resource);'
```

Without these policies (and with authentication enabled), all policy/metadata management endpoints and cross-user authorization queries will return HTTP 403.

When authentication is disabled (`ENABLE_AUTH=false`), meta permission checks are skipped entirely.

## Conformance Test Setup

The `conformance-test-setup` binary creates the metadata and policy needed by the conformance test on a running permission-service instance via its REST API.

```shell
cargo run --bin conformance-test-setup -- \
  --rest-url http://localhost:3000 \
  --principal conformance-test-principal \
  --action-name read \
  --action-service conformance-test \
  --resource-id conformance-test-resource \
  --resource-type ConformanceTestResource
```

This registers a service with an action and resource type, then creates a Cedar permit policy for the given principal/action/resource combination. The conformance test can then be run with matching parameters.

| CLI flag | Env var | Default |
|---|---|---|
| `--rest-url` | `REST_URL` | *(required)* |
| `--principal` | `PRINCIPAL` | `conformance-test-principal` |
| `--action-name` | `ACTION_NAME` | `read` |
| `--action-service` | `ACTION_SERVICE` | `conformance-test` |
| `--resource-id` | `RESOURCE_ID` | `conformance-test-resource` |
| `--resource-type` | `RESOURCE_TYPE` | `ConformanceTestResource` |
| `--wait-retries` | `WAIT_RETRIES` | `10` |
| `--wait-interval` | `WAIT_INTERVAL` | `5` |

## Environment variables

Use `cargo run --bin permission-service -- --help` to see help for environment variables or command line arguments.

## Releasing the service

1. Move all changes from "Unreleased" section in `CHANGELOG.md` to a new section with a new version number and ISO date (yyyy-mm-dd). Keep an empty "Unreleased" section for future changes in upcoming.
2. Update the service version in `Cargo.toml`.
3. Run `cargo build` to update `Cargo.lock` and confirm that everything builds correctly.
4. Push changes to the `main` branch with the version number as the commit message.
5. Check that the `main` branch pipeline passed.
6. Create a git tag from the `main` branch with the version number as the tag name.

## License generation

To generate `ATTRIBUTIONS.md`, run the following commands:

1. `cargo install dd-rust-license-tool`
2. `dd-rust-license-tool write`
3. `python tools/generate_attributions.py` (Python 3.10+ is required; the script is only in NVIDIA's repository)
