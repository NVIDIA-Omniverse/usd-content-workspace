# Thumbnail Deployment Architecture

This document summarizes how the thumbnail services are deployed in the
`storage-api-dev` environment, based on the Helm charts and deployment values in
`usd-storage-api-deployment`.

## Components

The thumbnail stack is deployed as two Helm releases:

- `thumbnail-service`: Rust HTTP and gRPC API service.
- `thumbnail-task-queue`: Python FastAPI API, Celery worker, and Flower dashboard.

The current dev deployment config is defined in:

- `usd-storage-api-deployment/environments/dev/thumbnail-service/values.yaml`
- `usd-storage-api-deployment/environments/dev/thumbnail-task-queue/values.yaml`
- `usd-storage-api-deployment/environments/dev/releases.conf`

## `thumbnail-service`

The `thumbnail-service` chart deploys one Kubernetes `Deployment` with a single
`service-server` container (running the `server` binary) that serves both ports:

- REST/HTTP on `3000`.
- gRPC on `50051`.

The container is exposed through one ClusterIP `Service` named
`thumbnail-service`, with separate `rest` and `grpc` ports.

Event consumption runs inside the same single container:

- The service container receives `NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT` whenever `notifications.eventConsumer.storageEventQueue.enabled=true`.
- Otherwise it receives `DISABLE_EVENT_CONSUMPTION=true`.

In dev, the service is configured with:

- `STORAGE_GRPC_ENDPOINT=storage-service:8011` (from root `storage.endpoint`)
- `NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT=event-consumer-service:50051`
- `NOTIFICATIONS_EVENT_AGGREGATION_ENDPOINT=event-aggregation-service:50051`
- `THUMBNAIL_PATH_TEMPLATE=https://examplestorage.blob.core.windows.net/example/thumbnails/{resource_address_md5}/{resource_identity_md5}/{width}x{height}/q{quality}/{resource_filename}.auto.{output_encoding}`
  (from root `thumbnailPathTemplate`)

`GENERATED_THUMBNAIL_PRINCIPAL_IDENTITY` is no longer chart-configurable. The Rust
binaries still accept it via env / `thumbnailService.extraVars`; chart support
returns once storage propagates writer principal (see ADR 0001).

When `httpProxy.enabled=true`, the chart creates Contour
`HTTPProxy` resources for both REST and gRPC traffic. The gRPC proxy uses `h2c`
to reach the internal service port.

### Two Deployments, one chart, two durable queues

The release is two Deployments rendered by one chart (`helm/thumbnail-service`
— the worker's own chart was folded into it; see
`openspec/changes/fold-task-worker-into-service-chart`), not one container:

| | `thumbnail-service` (orchestrator) | `thumbnail-task-worker` |
|---|---|---|
| Chart | `helm/thumbnail-service` (both) | `helm/thumbnail-service` (both) |
| Values | `thumbnailService.*` | `thumbnailWorker.*` |
| Root runtime/shared keys | `notifications.{eventConsumer,eventAggregation}`, `thumbnailPathTemplate`, `thumbnailSizes`, `thumbnailConfig`, `workerRuntime`, `image.{tag,pullPolicy,pullSecrets}`, `serviceAccount.{create,automount,annotations}`, `podLabels`, `podAnnotations`, `nodeSelector`, `tolerations`, `affinity`, `volumes`, `volumeMounts`, `podSecurityContext`, `securityContext`, `serviceIdentity`, `observability`, `storage`, `streaming` (no `common:` wrapper) | same |
| Container | `thumbnail-service`, running `/app/server` | `thumbnail-worker`, running `/app/task-worker` |
| Serves traffic | REST `3000`, gRPC `50051` | none; probes and `/metrics` on `8080` |
| Durable queue | StorageEventQueue — `omni.storage.created` / `.deleted` | WorkerEventQueue — `omni.thumbnail.generate` |
| An ACK means | generate work was durably published | the thumbnail was converted, uploaded, associated |
| Scaling signal | request volume | conversion CPU |

The two queue ids **must differ**. Queue ids are namespaced per principal and
both Deployments run as the same service account, so a distinct bare id
suffices — but sharing one would have the consumers contend for a single durable
session.

Only the orchestrator serves HTTP or gRPC traffic. The worker is reached by
nothing: it pulls work from WorkerEventQueue and calls out to storage and back to the
orchestrator. Sizing, replica counts, and rollouts are independent even though
both now render from the same chart in one release — see
`helm/thumbnail-service/README.md#task-worker` for the worker's memory model
and the redelivery mechanism it relies on.

### Recursion prevention and replica count

Recursion prevention (skipping storage `CREATE` events for thumbnails this service just generated)
relies on two checks. The `generated_by_thumbnail_pipeline` event marker and the optional
`GENERATED_THUMBNAIL_PRINCIPAL_IDENTITY` writer-principal check are **inert today**: storage does
not yet propagate the signals they depend on. That leaves template-output shape matching as the
only active guard — a stateless, pure function of the configured
root `thumbnailPathTemplate` and the event's address, independent of which replica
handles a given event. `thumbnailService.replicaCount` is not constrained by recursion-prevention
correctness. `GENERATED_THUMBNAIL_PRINCIPAL_IDENTITY` is no longer a Helm value (still injectable
via env / `extraVars`); chart support returns once storage propagates writer principal. See ADR
0001 (`docs/adr/0001-thumbnail-storage.md`) for the full loop-prevention design.

### Service-identity / outbound auth

Outbound calls (storage-service gRPC, event-consumer gRPC, event-aggregation
gRPC) authenticate with one of two mutually exclusive flows, gated by Helm
values under `serviceIdentity.*`:

- **In-process client-credentials** (`serviceIdentity.clientCredentials.enabled=true`).
  The service exchanges `CLIENT_ID` + `CLIENT_SECRET_FILE` for an access token
  at startup using the `CLIENT_CREDENTIALS_OPENID_CONFIGURATION` discovery
  document and refreshes the token itself. The chart mounts the secret keyed
  by `clientSecretRef.key` (default `CLIENT_SECRET`) at `/etc/secrets`. The
  Secret itself is operator-provided (`clientSecretRef.name`); the chart does
  not create one.
- **Sidecar-driven access-token file** (`serviceIdentity.accessTokenFile.enabled=true`).
  An operator-supplied `client-credentials-sidecar` container runs as a
  sidecar container in the pod and writes a token to a shared in-memory
  `emptyDir` volume (`token-exchange`, mounted at `/etc/secrets/access-token`).
  The service container consumes that file via `ACCESS_TOKEN_FILE_PATH`
  and re-stat it every `ACCESS_TOKEN_FILE_POLL_INTERVAL` looking for rotations.
  The chart skips emitting any Secret in this mode.

The chart's `service.validateServiceIdentity` helper enforces mutual
exclusivity at template time (`helm template`/`helm install` fails fast if
neither or both modes are enabled, or if `accessTokenFile.enabled` is set
without a provisioner sidecar container), and the binary also fails fast at
startup as a second line of defense. See `helm/thumbnail-service/README.md`
for the canonical env-var names and the migration table from the legacy
`SERVICE_ACCOUNT_*` / `TENANT_ID` / `TOKEN_URI` names.

## `thumbnail-task-queue`

The `thumbnail-task-queue` chart deploys three components:

- API pod: FastAPI service on port `8000`.
- Worker pod: Celery worker consuming `thumbnail_queue`.
- Flower pod: Celery monitoring UI on port `5555`.

The API and worker both receive PostgreSQL settings:

- `POSTGRES_HOST`
- `POSTGRES_PORT`
- `POSTGRES_USER`
- `POSTGRES_DBNAME`
- `POSTGRES_PASSWORD_FILE`

The API, worker, and Flower receive the RabbitMQ broker URL from the
`rabbitmq_uri` key in the externally-managed `thumbnail-task-queue-env`
Secret. The chart consumes this Secret via `envFrom: secretRef` and maps
`rabbitmq_uri` onto `CELERY_BROKER_URL` for the Celery application. This
mirrors the `event-aggregation-env` / `event-consumer-env` pattern used by
the notification services that share the same RabbitMQ broker.

The worker also calls:

- `storage-service` to read source files and write generated thumbnails.
- `thumbnail-service` to update thumbnail metadata or associations.

## PostgreSQL

The thumbnail task queue does not use the same PostgreSQL instance as
`permission-service` today.

Current dev setup:

- `permission-service` uses `postgresql-permissions-rw.postgresql.svc.cluster.local`
  with database `permissions-v3` and owner `permissions`.
- `thumbnail-task-queue` uses `postgresql-thumbnail-rw.postgresql.svc.cluster.local`
  with database `taskqueue` and owner `thumbnail`.

Both are CloudNativePG `Cluster` resources in the `postgresql` namespace, but
they are separate PostgreSQL clusters with separate primary services, pods,
users, databases, and secrets.

### Can Thumbnail Use The Permissions PostgreSQL Instance?

Technically yes, but it would be a deployment and database administration
change, not just an application toggle.

To share the permissions PostgreSQL cluster, the deployment would need:

- A separate thumbnail database or schema in `postgresql-permissions`.
- A dedicated thumbnail database user and password.
- Updated `thumbnail-task-queue` values for `postgres.host`, `postgres.user`,
  and `postgres.dbname`.
- Updated Kubernetes secrets for the thumbnail database password.
- A migration/bootstrap path for creating the task queue tables in the shared
  cluster.

Sharing the same PostgreSQL cluster may reduce infrastructure overhead, but it
also couples permissions and thumbnail task processing for capacity, backups,
upgrades, failover, and incident blast radius. Keeping a separate
`postgresql-thumbnail` cluster gives the task queue independent scaling and
operational isolation.

## RabbitMQ

RabbitMQ is shared infrastructure, but thumbnail task processing uses its own
vhost.

The infrastructure install creates one `notifications-rabbitmq` release in the
`storage-api` namespace. Its load definition creates these vhosts:

- `/`
- `notifications`
- `thumbnail`

The event services use the notifications vhost for storage and durable queue
events. The thumbnail task queue uses the thumbnail vhost for Celery task
messages; that AMQP URI is delivered to pods via the `rabbitmq_uri` key in
the `thumbnail-task-queue-env` Secret.

`create-secrets.sh` creates two Secrets for the task queue:

- `thumbnail-task-queue-env` (env-style, consumed via `envFrom: secretRef`):
  - `rabbitmq_uri` — AMQP connection URI for the thumbnail vhost.
- `thumbnail-task-queue-secrets` (file-mounted under `/etc/secrets`):
  - `POSTGRES_PASSWORD`
  - `SERVICE_ACCOUNT_CLIENT_SECRET`

If `THUMBNAIL_RABBITMQ_URI` is not set but `RABBITMQ_URI` is set, the script
derives the thumbnail broker URI by replacing the RabbitMQ vhost with
`/thumbnail`.
