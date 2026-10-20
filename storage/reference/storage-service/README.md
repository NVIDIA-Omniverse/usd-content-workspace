# Storage Service

## Release workflow

Creating a new release requires updating the `workspace.package.version` property in the `Cargo.toml` file to the desired version number, updating the `CHANGELOG.md` file, and pushing these changes into the `main` branch as a single merge request.

The changelog file must be updated to maintain the [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) format, so that the contents of the `[Unreleased]` version are migrated to the newly created version, rendering the former empty:

**Before**

```md
## [Unreleased]

### Added

- PROJ-123: Add some feature.

### Fixed

- PROJ-124: Fix some bug.
```

**After**

```md
## [Unreleased]

## [0.9.9] - 2077-01-01

### Added

- PROJ-123: Add some feature.

### Fixed

- PROJ-124: Fix some bug.
```

## Building the container image

From this directory:

```shell
docker build -t storage-service .
```

In the consolidated repository [NVIDIA-Omniverse/usd-content-workspace](https://github.com/NVIDIA-Omniverse/usd-content-workspace), the workspace has path dependencies outside its own directory (`storage/reference/storage-core`, `storage/reference/externals/azure-sdk` and `common/url-normalizer`), so the image is built with the repository root as the build context. From the root of that repository:

```shell
docker build -f storage/reference/storage-service/Dockerfile \
  --build-arg COMPONENT_DIR=storage/reference/storage-service -t storage-service .
```

The `github-publish:layout-test` jobs build and test that layout on every merge request.

## Artifact publishing

Artifacts are published with each version update to the `Cargo.toml` file. To temporarily compile and deploy artifacts from a different branch, you need to create a [semver-compatible](https://semver.org/) tag (such as `0.9.9` or `0.9.9-alpha.1`) and push it in the repository.

### Third-Party License Attribution

Tagged release containers automatically include third-party license attributions at `/licenses/`:
- `/licenses/attributions.md` - Full license texts for all dependencies
- `/licenses/license_links.md` - Verified links to license files in repositories

## OpenTelementry exporters

The Storage Service configures the OpenTelemetry (OTEL) runtime using [standardized environment variables](https://opentelemetry.io/docs/specs/otel/configuration/sdk-environment-variables/). Currently, only the "prometheus" and "otel" exporter types are supported.

## Testing

Most of the test scenarios run against physical services and require infrastructure to be bootstrapped first. While emulators and API implementations are readily available for many of them, specific tests have to be run against real services nevertheless: Azure Blob, Azure Service Bus, and Azure CosmosDB. When writing tests, prefer avoiding these backends unless absolutely necessary.

### Prerequisites

You will need the Azure CLI to bootstrap resources in Azure. Reference the [official manual](https://learn.microsoft.com/en-us/cli/azure/install-azure-cli) for installation instructions.

You will need JQ. Reference the [official manual](https://jqlang.org/download/) for installation instructions.

You will need Docker to run the local services. Reference the [official manual](https://docs.docker.com/engine/install/) for installation instructions.

### Set up a Resource Group (once)

Skip this step if you already have a resource group.

Resource groups are containers that group resources in a solution. It's recommended that you place all resources in a group to facilitate bookkeeping and being able to wipe the entire setup with a single command. Use the following command to create a resource group:

```shell
export AZURE_RG_NAME="<replace-with-your-resource-group-name>"
export AZURE_RG_LOCATION="<replace-with-your-location>"

az group create \
    --location "$AZURE_RG_LOCATION" \
    --name "$AZURE_RG_NAME"
```

### Set up Azure Blob (once)

Skip this step if you already have a storage container.

Run the following commands to create a storage account and a container in that account:

```shell
export AZURE_STORAGE_ACCOUNT_NAME="<replace-with-your-storage-account-name>"
export AZURE_STORAGE_CONTAINER_NAME="unittest"

az storage account create \
    --name "$AZURE_STORAGE_ACCOUNT_NAME" \
    --location "$AZURE_RG_LOCATION" \
    --resource-group "$AZURE_RG_NAME" \
    --access-tier Hot \
    --sku Standard_LRS

az storage container create \
    --name "$AZURE_STORAGE_CONTAINER_NAME" \
    --account-name "$AZURE_STORAGE_ACCOUNT_NAME" \
    --public-access off
```

Fetch the storage account key and remember it:

```shell
az storage account keys list -n "$AZURE_STORAGE_ACCOUNT_NAME" -g "$AZURE_RG_NAME" | jq -rM '.[] | select(.keyName == "key1") | .value'
```

### Set up Azure Cosmos DB (once)

Skip this step if you already have a Cosmos DB database.

Run the following commands to create a Cosmos DB account, database, and a container in that database:

```shell
export COSMOSDB_ACCOUNT="<replace-with-your-cosmosdb-account-name>"
export COSMOSDB_DATABASE_ID="unittests"
export COSMOSDB_CONTAINER_ID="unittests"

az cosmosdb create \
    --name "$COSMOSDB_ACCOUNT" \
    --resource-group "$AZURE_RG_NAME" \
    --default-consistency-level Strong \
    --capabilities EnableServerless \
    --locations \
        regionName="$AZURE_RG_LOCATION" \
        failoverPriority=0 \
        isZoneRedundant=False

az cosmosdb sql database create \
    --name "$COSMOSDB_DATABASE_ID" \
    --account-name "$COSMOSDB_ACCOUNT" \
    --resource-group "$AZURE_RG_NAME"

az cosmosdb sql container create \
    --name "$COSMOSDB_CONTAINER_ID" \
    --database-name "$COSMOSDB_DATABASE_ID" \
    --account-name "$COSMOSDB_ACCOUNT" \
    --resource-group "$AZURE_RG_NAME" \
    --partition-key-path "/resourceUriHash" \
    --idx '{"includedPaths":[{"path":"/resourceUriHash/?"}],"excludedPaths":[{"path":"/*"}]}'
```

Fetch the account key and remember it:

```shell
az cosmosdb keys list --name "$COSMOSDB_ACCOUNT" --resource-group "$AZURE_RG_NAME" | jq -rM '.primaryMasterKey'
```

### Set up Azure Service Bus (once)

Skip this step if you already have a Service Bus queue.

Create a service bus namespace and a queue in that namespace:

```shell
export AZURE_SERVICE_BUS_QUEUE_NAMESPACE="<replace-with-your-namespace-name>"
export AZURE_SERVICE_BUS_QUEUE_NAME="unittests"

az servicebus namespace create \
    --name "$AZURE_SERVICE_BUS_QUEUE_NAMESPACE" \
    --resource-group "$AZURE_RG_NAME" \
    --location "$AZURE_RG_LOCATION" \
    --sku Basic

az servicebus queue create \
    --resource-group "$AZURE_RG_NAME" \
    --namespace-name "$AZURE_SERVICE_BUS_QUEUE_NAMESPACE" \
    --name "$AZURE_SERVICE_BUS_QUEUE_NAME" \
    --lock-duration "PT30S" \
    --enable-session "false" \
    --enable-partitioning "false" \
    --duplicate-detection "false" \
    --default-message-time-to-live "PT60S"
```

Fetch the namespace key and remember it:

```shell
az servicebus namespace authorization-rule keys list \
  --resource-group "$AZURE_RG_NAME" \
  --namespace-name "$AZURE_SERVICE_BUS_QUEUE_NAMESPACE" \
  --name RootManageSharedAccessKey | jq -rM '.primaryKey'
```

### Create a reusable environment (once)

Create a file `scripts/env.sh`:

```shell
#!/bin/bash -e

script_dir="$(dirname "$(readlink -f "$0")")"

source "$script_dir/env-template.sh"

export COSMOSDB_ACCOUNT="<replace-with-your-account-name>"
export COSMOSDB_DATABASE_ID="<replace-with-your-database-name>"
export COSMOSDB_CONTAINER_ID="<replace-with-your-container-name>"
export COSMOSDB_PRIMARY_KEY="<replace-with-db-primary-key>"

export AZURE_SERVICE_BUS_QUEUE_NAMESPACE="<replace-with-your-queue-namespace-name>"
export AZURE_SERVICE_BUS_QUEUE_NAME="<replace-with-your-queue-name>"
export AZURE_SERVICE_BUS_POLICY_NAME="RootManageSharedAccessKey"
export AZURE_SERVICE_BUS_POLICY_KEY="<replace-with-your-policy-key>"

export AZURE_BLOB_STORAGE_ACCOUNT="<replace-with-your-storage-account-name>"
export AZURE_BLOB_STORAGE_KEY="<replace-with-your-storage-key>"
export AZURE_BLOB_STORAGE_CONTAINER="<replace-with-your-storage-container>"
```

### Import environment variables

Import settings from the `scripts/env.sh` file:

```shell
source ./scripts/env.sh
```

### Bootstrap local services

Spawn containers and initialize them:

```shell
./scripts/local-bootstrap.sh
```

### Run tests

| Run a command                                                              | To execute ...                                                                           |
| -------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| `cargo test --features test -- --nocapture`                                | ... all tests                                                                            |
| `cargo test --features test --bin storage-service -- --nocapture`          | ... all unit tests in the `storage-service` crate                                        |
| `cargo test --features test --bin storage-service mytest -- --nocapture`   | ... all unit tests whose names contain "mytest" in the `storage-service` crate           |
| `cargo test --features test --test metadata_backend -- --nocapture`        | ... all integration tests from the `metadata_backend.rs` file                            |
| `cargo test --features test --test metadata_backend mytest -- --nocapture` | ... all integration tests whose names contain "mytest" in the `metadata_backend.rs` file |

### Shutdown local services

Shutdown containers:

```shell
./scripts/local-teardown.sh
```

## Documentation

The documentation sources are under [`docs/`](docs/), written in reStructuredText
and readable as-is. NVIDIA builds the rendered documentation site from these
sources internally; where the documentation build tooling is present,
`make docs` builds it.
