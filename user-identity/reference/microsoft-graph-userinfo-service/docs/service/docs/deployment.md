(userinfo-service-deployment-configuration)=
# User Info Service Configuration

**Goal**: Deploy the Microsoft Graph User Info Service in Kubernetes. The recommended deployment uses a
single writer to synchronize Microsoft Entra ID into PostgreSQL and one or more readers to serve REST and gRPC
requests. A reader-only mode that queries Microsoft Graph directly is also available for initial testing.

(userinfo-service-deployment-service-ports)=
## Service Ports

| Port  | Protocol  | Description       |
|-------|-----------|-------------------|
| 8080  | HTTP/REST | REST API endpoint |
| 50051 | gRPC      | gRPC API endpoint |

The health and readiness endpoints are available on the REST port at `/healthz` and `/readyz`.

(userinfo-service-deployment-prerequisites)=
## Prerequisites

Before deploying the service, prepare:

- A Kubernetes cluster with Helm 3 and `kubectl`.
- An NGC API key that can pull the Helm chart and container images.
- A [Microsoft Entra app registration](https://learn.microsoft.com/en-us/entra/identity-platform/quickstart-register-app)
  with a tenant ID, client ID, and either a client secret or Azure Workload Identity federation.
- Microsoft Graph application permissions with tenant administrator consent:
  - `User.Read.All`
  - `Group.Read.All`
  - `GroupMember.Read.All` when group membership queries or synchronization are enabled.
- A PostgreSQL database for the recommended synchronized directory deployment.

The service uses the OAuth 2.0 client credentials flow and requests
`https://graph.microsoft.com/.default`. Configure application permissions, rather than delegated permissions,
because no signed-in user participates in this flow.

(userinfo-service-deployment-pulling-the-helm-chart)=
## Pulling the User Info Service Helm Chart

Pull and unpack the User Info Service Helm chart from the NGC catalog. Replace `{NGC_API_KEY}` and `{VERSION}`
with the values for your deployment.

```bash
helm fetch https://helm.ngc.nvidia.com/nvidia/omniverse/charts/microsoft-graph-userinfo-service-{VERSION}.tgz --username='$oauthtoken' --password=${NGC_API_KEY}
tar -xvf microsoft-graph-userinfo-service-{VERSION}.tgz
cd microsoft-graph-userinfo-service
```

(userinfo-service-deployment-create-namespace)=
## Creating the Namespace

This guide uses the `storage-apis` namespace.

```bash
kubectl create namespace storage-apis
```

Ensure that an image-pull secret named `ngcpull-secret` exists in this namespace before installing the chart.

(userinfo-service-deployment-entra-credentials)=
## Preparing Microsoft Entra Credentials

For client-secret authentication, create a Kubernetes Secret containing the app registration's client secret:

```bash
kubectl create secret generic microsoft-graph-userinfo-service-az-credentials \
  --from-literal=client-secret="{AZURE_CLIENT_SECRET}" \
  --namespace storage-apis
```

Keep the tenant ID and client ID for the Helm values file. The chart reads the client secret from the Secret and
does not require it to be stored in the values file.

For Azure Workload Identity, set `providers.entra.authMode` to `workload-identity`, annotate the chart's service
account for the federated identity, and omit the client-secret Secret. The workload identity setup itself depends
on the Kubernetes cluster and Entra tenant configuration.

(userinfo-service-deployment-postgresql)=
## Setting Up PostgreSQL

The synchronized directory deployment requires a PostgreSQL snapshot store shared by the writer and all readers.
The writer synchronizes directory data into the store, and readers periodically reload the stored snapshot into
memory.

(userinfo-service-deployment-azure-postgresql)=
### Recommended: Azure Database for PostgreSQL

For production deployments on Azure, use
[Azure Database for PostgreSQL flexible server](https://learn.microsoft.com/en-us/azure/postgresql/flexible-server/overview)
instead of operating PostgreSQL inside the Kubernetes cluster. It provides managed patching, automated backups,
high-availability options, monitoring, and TLS-encrypted connections.

Follow Microsoft's
[creation quickstart](https://learn.microsoft.com/en-us/azure/postgresql/flexible-server/quickstart-create-server-portal)
to provision a flexible server. Create a dedicated `microsoft_graph_userinfo_service` database user and make it
the owner of the `graph-userinfo` database. See
[Manage users](https://learn.microsoft.com/en-us/azure/postgresql/flexible-server/how-to-create-users) for
Microsoft's database and user creation guidance.

For production, prefer Microsoft Entra authentication with a short-lived access token over a static database
password: the service reads a rotating token from a file that a token-provider sidecar keeps current, so no
long-lived password is stored. The static-password setup shown below is a simpler starting point. The
Database Configuration page in the User Info Service documentation covers both credential options and the
Azure-specific guidance. When using a static password, store it in the Kubernetes Secret described in
{ref}`userinfo-service-deployment-postgresql-secret`.

Choose a networking configuration that allows the Kubernetes cluster to reach the database. For an Azure-hosted
production cluster, prefer private access through virtual network integration. See
[Networking overview](https://learn.microsoft.com/en-us/azure/postgresql/flexible-server/concepts-networking)
for the available private and public connectivity options.

Use the flexible server endpoint and require TLS in `userinfo-values.yaml`:

```yaml
providers:
  cache:
    postgres:
      enabled: true
      host: "{SERVER_NAME}.postgres.database.azure.com"
      port: 5432
      dbname: "graph-userinfo"
      user: "microsoft_graph_userinfo_service"
      password:
        secretRef:
          name: "microsoft-graph-userinfo-service-pg-credentials"
          key: "password"
      sslMode: "require"
```

Azure Database for PostgreSQL requires TLS for client connections. See Microsoft's
[TLS documentation](https://learn.microsoft.com/en-us/azure/postgresql/flexible-server/security-tls) for
supported protocol versions and certificate guidance.

(userinfo-service-deployment-cloudnativepg)=
### Alternative: In-Cluster PostgreSQL with CloudNativePG

[CloudNativePG](https://cloudnative-pg.io/) is a Kubernetes operator that manages PostgreSQL clusters.

(userinfo-service-deployment-install-cloudnativepg)=
#### Install the CloudNativePG Operator

```bash
helm repo add cnpg https://cloudnative-pg.github.io/charts
helm upgrade --install cnpg-operator cnpg/cloudnative-pg \
  --namespace cnpg-system \
  --create-namespace
kubectl get pods -n cnpg-system
```

(userinfo-service-deployment-create-postgresql-cluster)=
#### Create a PostgreSQL Cluster

Save the following manifest as `userinfo-pg-cluster.yaml`:

```yaml
apiVersion: postgresql.cnpg.io/v1
kind: Cluster
metadata:
  name: userinfo-pg
  namespace: storage-apis
spec:
  instances: 1

  bootstrap:
    initdb:
      database: graph-userinfo
      owner: microsoft_graph_userinfo_service

  storage:
    size: 10Gi
```

Apply the manifest and wait for the cluster to become ready:

```bash
kubectl apply -f userinfo-pg-cluster.yaml
kubectl get cluster userinfo-pg -n storage-apis
```

CloudNativePG creates the `userinfo-pg-app` Secret containing the generated application credentials and the
`userinfo-pg-rw` Service for read-write database connections.

(userinfo-service-deployment-postgresql-secret)=
### Preparing the PostgreSQL Secret

The Helm chart expects a Secret key containing the PostgreSQL password. Copy the generated password into the
Secret name and key used by the chart:

```bash
PGPASSWORD=$(kubectl get secret userinfo-pg-app -n storage-apis -o jsonpath='{.data.password}' | base64 -d)
kubectl create secret generic microsoft-graph-userinfo-service-pg-credentials \
  --from-literal=password="$PGPASSWORD" \
  --namespace storage-apis
```

If PostgreSQL is managed outside CloudNativePG, create the Secret directly:

```bash
kubectl create secret generic microsoft-graph-userinfo-service-pg-credentials \
  --from-literal=password="{POSTGRES_PASSWORD}" \
  --namespace storage-apis
```

Any PostgreSQL-compatible deployment can be used. Set its host, port, database, user, and TLS mode in the Helm
values.

(userinfo-service-deployment-synchronized-directory)=
## Synchronized Directory Deployment

Create `userinfo-values.yaml` with the Entra and PostgreSQL configuration. Replace the tenant and client IDs with
the values from the Entra app registration.

```yaml
image:
  pullSecrets:
    - name: ngcpull-secret

providers:
  entra:
    tenantId: "{AZURE_TENANT_ID}"
    clientId: "{AZURE_CLIENT_ID}"
    authMode: "client-secret"
    clientSecret:
      secretRef:
        name: "microsoft-graph-userinfo-service-az-credentials"
        key: "client-secret"

  cache:
    enabled: true
    users:
      enabled: true
    groups:
      enabled: true
    memberships:
      enabled: false
    syncInterval: "5m"

    postgres:
      enabled: true
      host: "userinfo-pg-rw"
      port: 5432
      dbname: "graph-userinfo"
      user: "microsoft_graph_userinfo_service"
      password:
        secretRef:
          name: "microsoft-graph-userinfo-service-pg-credentials"
          key: "password"
      sslMode: "disable"

writer:
  enabled: true

reader:
  enabled: true
  replicaCount: 1
  reloadInterval: "60s"
```

`userinfo-values.yaml`

The writer always runs as one replica. Scale read capacity with `reader.replicaCount`, or set
`autoscaling.enabled` to hand the reader count to an externally managed autoscaler.

Membership synchronization is disabled in this example because it can substantially increase initial
synchronization time and memory use for large directories. Enable it when the membership endpoints must be
served from the local snapshot:

```yaml
providers:
  cache:
    memberships:
      enabled: true
```

Increase `writer.resources` and `reader.resources` when synchronizing a large directory.

(userinfo-service-deployment-install-synchronized-directory)=
### Install the Synchronized Directory Deployment

Validate and dry-run the chart:

```bash
helm template . -f userinfo-values.yaml
helm upgrade --install userinfo-service . \
  -f userinfo-values.yaml \
  --namespace storage-apis \
  --dry-run \
  --debug
```

Install the service:

```bash
helm upgrade --install userinfo-service . \
  -f userinfo-values.yaml \
  --namespace storage-apis
kubectl get pods -n storage-apis
```

The writer becomes ready once it holds a complete directory: immediately when PostgreSQL already holds one,
otherwise after its first full synchronization completes. Readers become ready as soon as they start and load
whatever PostgreSQL holds, without waiting for the writer, so right after a first installation they serve an
empty or partial directory until the writer has saved a complete one and the readers reload it. The Kubernetes
Service routes API traffic only to reader pods.

(userinfo-service-deployment-live-mode)=
## Reader-Only Live Graph Mode

For initial testing without PostgreSQL, disable the writer and cache. The reader sends each directory request to
Microsoft Graph.

```yaml
image:
  pullSecrets:
    - name: ngcpull-secret

providers:
  entra:
    tenantId: "{AZURE_TENANT_ID}"
    clientId: "{AZURE_CLIENT_ID}"
    authMode: "client-secret"
    clientSecret:
      secretRef:
        name: "microsoft-graph-userinfo-service-az-credentials"
        key: "client-secret"

  cache:
    enabled: false
    postgres:
      enabled: false

writer:
  enabled: false

reader:
  enabled: true
```

`userinfo-values.yaml`

Install this mode with the same validation and `helm upgrade --install` commands used for the synchronized
directory deployment. This mode removes the PostgreSQL dependency but makes API availability and latency
dependent on Microsoft Graph.

(userinfo-service-deployment-verification)=
## Verifying the Deployment

Check the workloads and probe the in-cluster service:

```bash
kubectl get deployments,pods,service -n storage-apis
kubectl run userinfo-health-check \
  --rm \
  --restart=Never \
  --namespace storage-apis \
  --image=curlimages/curl \
  -- http://userinfo-service.storage-apis.svc.cluster.local:8080/healthz
kubectl run userinfo-readiness-check \
  --rm \
  --restart=Never \
  --namespace storage-apis \
  --image=curlimages/curl \
  -- http://userinfo-service.storage-apis.svc.cluster.local:8080/readyz
```

A successful `/healthz` response confirms that the process is running. A successful `/readyz` response confirms
that a reader is serving. In the synchronized directory deployment it does not confirm that the directory is
populated: a reader may serve an empty or partial directory until the writer's first synchronization has been
saved. Confirm that the writer pod is ready and that a user listing returns the expected users; see
{ref}`Readiness and Health Signals <userinfo-service-observability-readiness>` for the metrics that show how
much data each reader holds.

(userinfo-service-deployment-smoke-test)=
### Running the Helm Smoke Test

The chart includes an optional Helm test that exercises the REST and gRPC APIs. Enable it in
`userinfo-values.yaml`:

```yaml
smokeTest:
  enabled: true
```

After upgrading the release, run:

```bash
helm test userinfo-service -n storage-apis --logs
```

If an authentication gateway protects the service, also configure `serviceIdentity` with credentials that
produce a token for the User Info Service audience.

(userinfo-service-deployment-permission-service)=
## Permission Service Integration

Authorization through the Permission Service is disabled by default. To enable it, configure the Permission
Service base URL and the service name used in authorization requests:

```yaml
permission:
  enabled: true
  url: "http://permission-service.storage-apis.svc.cluster.local:3000/"
  serviceName: "userinfo"
  cacheTtl: "5m"
  cacheMaxCapacity: 10000
```

The Permission Service must contain policies and service metadata for the configured service name before this
integration is enabled. If the Permission Service cannot evaluate a request, the User Info Service fails closed.
Health, readiness, and current-user requests are exempt from permission checks.

(userinfo-service-deployment-ingress)=
## Ingress Access

Enable the chart's `httpProxy` configuration to expose the REST and gRPC APIs through
[Contour HTTPProxy](https://projectcontour.io/docs/):

```yaml
httpProxy:
  enabled: true
  fqdn:
    host: "userinfo"
    domain: "{DNS_DOMAIN}"
  tls:
    enabled: true
    secretName: "{TLS_SECRET_NAME}"
```

Upgrade the release and verify the HTTPProxy resources:

```bash
helm upgrade --install userinfo-service . \
  -f userinfo-values.yaml \
  --namespace storage-apis
kubectl get httpproxy -n storage-apis
```

The chart creates two HTTPProxy objects:

- REST — FQDN `<host>.<domain>`, routes to the REST port
- gRPC — FQDN `grpc.<host>.<domain>`, routes to the gRPC port with HTTP/2 cleartext
  (`h2c`) to the backend

When TLS and CertManager are enabled, the certificate covers both hostnames. Use
`httpProxy.rest` and `httpProxy.grpc` to override shared settings (for example timeouts)
per protocol.

(userinfo-service-deployment-clean-up)=
## Clean Up

Remove only resources created by this guide:

```bash
helm uninstall userinfo-service -n storage-apis
kubectl delete secret microsoft-graph-userinfo-service-az-credentials -n storage-apis
kubectl delete secret microsoft-graph-userinfo-service-pg-credentials -n storage-apis
kubectl delete cluster userinfo-pg -n storage-apis
kubectl delete namespace storage-apis
```

(userinfo-service-deployment-references)=
## References

- [Microsoft Graph permissions reference](https://learn.microsoft.com/en-us/graph/permissions-reference)
- [Microsoft identity platform client credentials flow](https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-client-creds-grant-flow)
- [Azure Database for PostgreSQL flexible server overview](https://learn.microsoft.com/en-us/azure/postgresql/flexible-server/overview)
- [Create an Azure Database for PostgreSQL flexible server](https://learn.microsoft.com/en-us/azure/postgresql/flexible-server/quickstart-create-server-portal)
- [Manage Azure Database for PostgreSQL users](https://learn.microsoft.com/en-us/azure/postgresql/flexible-server/how-to-create-users)
- [Azure Database for PostgreSQL networking](https://learn.microsoft.com/en-us/azure/postgresql/flexible-server/concepts-networking)
- [Azure Database for PostgreSQL TLS](https://learn.microsoft.com/en-us/azure/postgresql/flexible-server/security-tls)
- [CloudNativePG documentation](https://cloudnative-pg.io/docs/)
- [Contour HTTPProxy documentation](https://projectcontour.io/docs/)

(userinfo-service-deployment-see-next)=
## See next

- {ref}`Database Configuration <userinfo-service-database-database-configuration>`
