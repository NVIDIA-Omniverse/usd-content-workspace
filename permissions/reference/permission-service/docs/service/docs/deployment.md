(permission-service-deployment-permission-service-configuration)=
# Permission Service Configuration

**Goal**: Deploy the Permission Service in Kubernetes. The service supports two modes: **config-file** mode
(read-only, no database required) for quick testing, and **Postgres** mode for full policy management. This guide
starts with config-file mode as the simplest way to get running, then covers upgrading to Postgres.

(permission-service-deployment-service-ports)=
## Service Ports

| Port | Protocol  | Description       |
|------|-----------|-------------------|
| 3000 | HTTP/REST | REST API endpoint |
| 3010 | gRPC      | gRPC API endpoint |

(permission-service-deployment-pulling-the-permission-service-helm-chart)=
## Pulling the Permission Service Helm Chart

Pull and unpack the Permission Service Helm chart from the NGC catalog. Replace `{NGC_API_KEY}` with your NGC API
key.

```bash
# pull the chart from NGC
helm fetch https://helm.ngc.nvidia.com/nvidia/omniverse/charts/permission-service-{VERSION}.tgz --username='$oauthtoken' --password=${NGC_API_KEY}

# unpack the chart and cd into the directory
tar -xvf permission-service-{VERSION}.tgz
cd permission-service
```

(permission-service-deployment-config-file-mode)=
## Config File Mode

Config-file mode is the simplest way to start the Permission Service. Policies are defined as YAML directly in
Helm values and mounted into the container — no external database is required.

(permission-service-deployment-setting-up-default-policies)=
### Setting Up Default Policies

Create a `permission-values.yaml` file to configure the service in config-file mode with default policies.

```bash
# create and edit the local values file, we're using VS Code for this example
code permission-values.yaml
```

Add the image-pull secret and configure the database type:

```yaml
image:
  pullSecrets:
    - name: ngcpull-secret

database:
  type: "config-file"

  init:
    policies:
      # Permit all requests — useful for initial testing
      - policy: "permit(principal, action, resource);"
```

permission-values.yaml

Policies are written in [Cedar](https://www.cedarpolicy.com/) policy language. Each entry under
`database.init.policies` must contain exactly one Cedar policy statement in the `policy` field. See the
[official Cedar documentation](https://docs.cedarpolicy.com/) for the full language reference.

```{note}
In config-file mode, the service reloads a policy file whenever its modification time changes. When you change
`database.init.policies` and run `helm upgrade`, the chart's `checksum/config-files` pod annotation changes with the
rendered ConfigMap, so the Deployment performs a rolling update and the new pods start with the updated policies; no
manual restart is required.
```

(permission-service-deployment-granting-administrative-permissions)=
### Granting Administrative Permissions

Access to the policy management and metadata APIs is gated by **meta-permissions** — Cedar actions on the reserved
`permissions` service that every authenticated caller is evaluated against. If none of these actions are granted
to anyone, the service starts up correctly but **no caller can list, create, or delete policies, change service
metadata, or inspect other users' effective permissions**.

| Action                     | Protects                                                                                   |
|----------------------------|--------------------------------------------------------------------------------------------|
| `permissions:view`         | Reading policies through `GET /v1beta/policies/` and `GET /v1beta/policies/{id}`, and authorizing requests whose `principal` differs from the caller. |
| `permissions:edit`         | Creating and deleting policies through `PUT /v1beta/policies/`, `PUT /v1beta/policies/batch/`, and `DELETE /v1beta/policies/{id}`. |
| `permissions:meta:read`    | Reading service metadata through the `GET` endpoints under `/v1beta/services/`.            |
| `permissions:meta:write`   | Writing service metadata through the `PUT` and `DELETE` endpoints under `/v1beta/services/`. |
| `permissions:diagnostics`  | Calling `POST /v1beta/diagnostics/authorize/`.                                             |

```{note}
Meta-permissions are only enforced when authentication is enabled (`openId.enabled: true`). When authentication is
disabled, every endpoint is reachable without a token and these actions are not evaluated.
```

Meta-permissions are granted the same way as any other permission — by writing Cedar policies that `permit` them
for the principals who should have access. Seed those policies through `database.init.policies` alongside the
application policies for the services you deploy. For example, to give the single user `alice` full
administrative access:

```yaml
database:
  init:
    policies:
      # Administrative access for one user
      - policy: 'permit(principal == Principal::"alice", action == Action::"permissions:view", resource);'
      - policy: 'permit(principal == Principal::"alice", action == Action::"permissions:edit", resource);'
      - policy: 'permit(principal == Principal::"alice", action == Action::"permissions:meta:read", resource);'
      - policy: 'permit(principal == Principal::"alice", action == Action::"permissions:meta:write", resource);'
      - policy: 'permit(principal == Principal::"alice", action == Action::"permissions:diagnostics", resource);'
```

In practice, most deployments grant these actions to a group rather than a specific user, for example:

```yaml
database:
  init:
    policies:
      # Administrative access for anyone whose token carries the "permission-admins" group claim
      - policy: >-
          permit(principal, action == Action::"permissions:view", resource)
          when { principal.groups.contains("permission-admins") };
      - policy: >-
          permit(principal, action == Action::"permissions:edit", resource)
          when { principal.groups.contains("permission-admins") };
      - policy: >-
          permit(principal, action == Action::"permissions:meta:read", resource)
          when { principal.groups.contains("permission-admins") };
      - policy: >-
          permit(principal, action == Action::"permissions:meta:write", resource)
          when { principal.groups.contains("permission-admins") };
      - policy: >-
          permit(principal, action == Action::"permissions:diagnostics", resource)
          when { principal.groups.contains("permission-admins") };
```

The grants above are deliberately broad (unconstrained `resource`). To scope `permissions:edit` or `permissions:view` to specific resources or folder subtrees, or to refine any meta-permission with `context.principal`, `context.action`, or `context.group`, see {ref}`Meta-permissions <permission-service-policy-language-meta-permissions>`.

The `principal.groups` attribute used above, along with any other claim-based attributes referenced in policies,
is populated from the caller's bearer token claims at request time.

```{warning}
The fully-open starter policy `permit(principal, action, resource);` shown above grants **all** actions, including
the four meta-permissions, to every authenticated caller. It is convenient for initial testing but should be
replaced with scoped policies before exposing the service to real users.
```

(permission-service-deployment-setting-up-service-metadata)=
### Setting Up Service Metadata

In addition to policies, the Helm value `database.init.services` seeds **service metadata** — the catalog of services
that integrate with authorization, the actions each exposes, the resource types those actions operate on, and the
per-service `idClaim` and per-resource-type `evaluationPriority` settings the service consumes at evaluation time.
The chart renders the list into a ConfigMap and mounts it at `/etc/permission-config/` so the service can load it
on startup.

The example below registers a single `storage-service` with two actions and two resource types:

```yaml
database:
  init:
    services:
      - name: "storage-service"
        principal:
          idClaim: "sub"
        actions:
          - "read"
          - "write"
        resourceTypes:
          - type: "object"
            evaluationPriority: "permit"
          - type: "folder"
            evaluationPriority: "permit"
```

Schema notes:

- `name` is the service identifier; actions are referenced from Cedar policies as `Action::"<name>:<action>"`.
- `principal.idClaim` is optional. When omitted, the service falls back to the deployment-wide claim set by
  `openId.principalIdClaim` (default `sub`). It names the bearer-token claim used as the principal id for requests targeting this service.
- `actions` are plain action names; each becomes addressable as `Action::"<service>:<name>"` in Cedar.
- `resourceTypes[].type` is the Cedar entity type used as `<type>::"<id>"` in policies.
- `resourceTypes[].evaluationPriority` is optional and defaults to `"forbid"`; the alternative is `"permit"`. It
  controls how competing `permit`/`forbid` policies are combined for resources of that type.
- A service that registers actions only (no resource types) simply omits `resourceTypes`; Cedar policies for those
  actions leave the `resource` slot unconstrained.

In Postgres mode, seeded entries are written into the database on startup and can be changed afterwards through
the metadata REST API under `/v1beta/services/`.

(permission-service-deployment-limitations)=
### Limitations

Config-file mode is **read-only**. Every request that would create, update, or delete a policy or service metadata
record is rejected with `501 Not Implemented` and a "not supported" message. This mode is suitable for testing and
static deployments where policies do not change at runtime.

To manage policies dynamically through the API, use Postgres mode.

(permission-service-deployment-install-the-permission-service-config-file-mode)=
### Install the Permission Service (Config File Mode)

Validate and install the chart:

```bash
# validate the chart
helm template . -f permission-values.yaml

# dry-run the install
helm upgrade --install permission-service . -f permission-values.yaml --namespace storage-apis --dry-run --debug
```

If everything looks good, install the Permission Service:

```bash
# install the permission service
helm upgrade --install permission-service . -f permission-values.yaml --namespace storage-apis

# validate the pod is running, this may take a few minutes to start up
kubectl get pods -n storage-apis
```

Verify the service is responding:

```bash
# test the permission service health endpoint
curl http://permission-service.storage-apis.svc.cluster.local:3000/health
```

(permission-service-deployment-postgres-mode)=
## Postgres Mode

Postgres mode unlocks full policy management through the API: create, update, and delete policies and service
metadata at runtime. Policies defined in `database.init.policies` are loaded as system policies on startup, and
service metadata defined in `database.init.services` is written into the database on startup and can be changed
afterwards through the metadata REST API exposed under `/v1beta/services/`.

```{note}
The {ref}`meta-permissions <permission-service-deployment-granting-administrative-permissions>` that gate the policy management, metadata, and
diagnostics APIs apply in Postgres mode as well. Seed them through `database.init.policies` on first deployment so
that administrators can reach those APIs once the service is up; otherwise the APIs are inaccessible even with a
valid bearer token.
```

(permission-service-deployment-setting-up-postgresql-with-cloudnativepg)=
### Setting Up PostgreSQL with CloudNativePG

[CloudNativePG](https://cloudnative-pg.io/) is a Kubernetes operator that manages the full lifecycle of
PostgreSQL clusters. It provides a convenient and easy-to-set-up way to run PostgreSQL directly in a
Kubernetes cluster.

(permission-service-deployment-install-the-cloudnativepg-operator)=
#### Install the CloudNativePG Operator

```bash
# add the CloudNativePG Helm repository
helm repo add cnpg https://cloudnative-pg.github.io/charts

# install the operator
helm upgrade --install cnpg-operator cnpg/cloudnative-pg \
  --namespace cnpg-system \
  --create-namespace
```

Verify the operator is running:

```bash
kubectl get pods -n cnpg-system
```

(permission-service-deployment-create-a-postgresql-cluster)=
#### Create a PostgreSQL Cluster

Create a `Cluster` custom resource to provision a PostgreSQL instance. Save the following as
`permission-pg-cluster.yaml`:

```yaml
apiVersion: postgresql.cnpg.io/v1
kind: Cluster
metadata:
  name: permission-pg
  namespace: storage-apis
spec:
  instances: 1

  bootstrap:
    initdb:
      database: permissions
      owner: permission-service

  storage:
    size: 1Gi
```

Apply the resource:

```bash
kubectl apply -f permission-pg-cluster.yaml

# wait for the cluster to become ready
kubectl get cluster -n storage-apis
```

CloudNativePG automatically creates a secret named `permission-pg-app` in the same namespace containing the
connection credentials (`host`, `port`, `user`, `password`, `dbname`). You can inspect it:

```bash
kubectl get secret permission-pg-app -n storage-apis -o yaml
```

(permission-service-deployment-prepare-secrets-for-the-permission-service)=
### Prepare Secrets for the Permission Service

The Helm chart does not create a secret for the Postgres credentials. Create a Kubernetes secret before
installing the chart and reference it with `database.postgres.secretRef.name` (default:
`permission-service-secret`). The secret holds these keys:

| Key                 | Required when                                     | Description                                                                              |
|---------------------|---------------------------------------------------|------------------------------------------------------------------------------------------|
| `POSTGRES_PASSWORD` | `database.postgres.tokenFile.enabled` is `false` | Postgres password, mounted into the container at `/etc/secrets/POSTGRES_PASSWORD`.       |
| `POSTGRES_USER`     | `database.postgres.user` is empty                 | Postgres username. When `database.postgres.user` is set, the chart uses that value instead. |

The chart does not track the content of this secret, so updating it does not roll the pods on the next
upgrade. Restart the deployment after changing the credentials.

You can create the secret from the CloudNativePG-generated credentials:

```bash
# extract the password from the CNPG-generated secret
PGPASSWORD=$(kubectl get secret permission-pg-app -n storage-apis -o jsonpath='{.data.password}' | base64 -d)

# create the permission service secret
kubectl create secret generic permission-service-secret \
  --from-literal=POSTGRES_PASSWORD="$PGPASSWORD" \
  --namespace storage-apis
```

Alternatively, if you manage PostgreSQL outside of CloudNativePG, create the secret directly with your database
password:

```bash
kubectl create secret generic permission-service-secret \
  --from-literal=POSTGRES_PASSWORD="{YOUR_POSTGRES_PASSWORD}" \
  --namespace storage-apis
```

(permission-service-deployment-cloudnativepg-alternatives)=
### CloudNativePG Alternatives

Any PostgreSQL-compatible database works with the Permission Service. If you prefer not to run PostgreSQL inside
the cluster, consider a cloud-managed service:

- **AWS**: [Amazon RDS for PostgreSQL](https://aws.amazon.com/rds/postgresql/) or
  [Amazon Aurora PostgreSQL](https://aws.amazon.com/rds/aurora/)
- **Azure**: [Azure Database for PostgreSQL](https://azure.microsoft.com/en-us/products/postgresql/)
- **GCP**: [Cloud SQL for PostgreSQL](https://cloud.google.com/sql/docs/postgres)

When using a managed service, provide the connection details (`host`, `port`, `user`, `dbname`, `sslMode`) in
the Helm values and create a Kubernetes secret with the password as described above.

(permission-service-deployment-deploy-in-postgres-mode)=
### Deploy in Postgres Mode

Create or update `permission-values.yaml` with Postgres settings. Replace the `host` with the service name of
your PostgreSQL instance (CloudNativePG creates a service named `permission-pg-rw` by default):

```yaml
image:
  pullSecrets:
    - name: ngcpull-secret

database:
  type: "postgres"

  init:
    policies:
      # System policies loaded on startup — optional
      - policy: "permit(principal, action, resource);"

  postgres:
    host: "permission-pg-rw"
    port: 5432
    user: "permission-service"
    secretRef:
      name: permission-service-secret
    dbname: "permissions"
    sslMode: ""
    connections: 16
    workerThreads: 32
```

permission-values.yaml

```{note}
The secret named by `database.postgres.secretRef.name` must exist before you install the chart; see
{ref}`Prepare Secrets for the Permission Service <permission-service-deployment-prepare-secrets-for-the-permission-service>`.
```

Validate and install:

```bash
# validate the chart
helm template . -f permission-values.yaml

# dry-run the install
helm upgrade --install permission-service . -f permission-values.yaml --namespace storage-apis --dry-run --debug
```

If everything looks good, install the Permission Service:

```bash
# install the permission service
helm upgrade --install permission-service . -f permission-values.yaml --namespace storage-apis

# validate the pod is running
kubectl get pods -n storage-apis
```

Verify the service is responding:

```bash
# test the permission service health endpoint
curl http://permission-service.storage-apis.svc.cluster.local:3000/health
```

(permission-service-deployment-url-remapper)=
## URL remapper

URL remapping is **off by default** (`config.urlRemapper.enabled: false`). When disabled, resource identifiers in authorization requests, policy writes, policy queries, and diagnostics pass through unchanged.

When enabled, the service rewrites resource URLs using a map of source URL prefix to target URL prefix before policies are stored, retrieved, or evaluated. The **longest matching source prefix** wins. Rules are matched against the canonical form of the id: `.` and `..` path segments, including their percent-encoded forms such as `%2e%2e`, are resolved and the path is URL-encoded the same way as for policy storage, so `omniverse://host/NVIDIA/../Projects/scene.usd` is remapped by the rule for `omniverse://host/Projects/`. Source prefixes are compared in the same canonical form. Use the same rules as Storage Service when both services must agree on canonical resource ids — the Helm block mirrors Storage Service's `config.urlRemapper` shape so one values fragment can be shared.

Remapping applies to resource ids in incoming REST and gRPC authorization payloads, to resource scopes inferred when policies are written, and to meta-permission checks that compare resource heads. Policies and requests should use the same URL form your clients send; the service normalizes to the canonical (post-remap) id for storage and matching.

| Setting | Helm value (`config.urlRemapper.*`) | Default | Description |
|---|---|---|---|
| Enable remapping | `enabled` | `false` | Turns prefix rewriting on for resource ids. |
| Inline rules | `rewriteRules` | `{}` | Map of source prefix → target prefix, given as a string or as an object with a `target` field. Used when `rewriteRulesRef.name` is empty. |
| External rules ConfigMap | `rewriteRulesRef.name` | `""` | When set, mount this ConfigMap instead of rendering chart-owned rules. |
| Key in external ConfigMap | `rewriteRulesRef.key` | `"url-remapper.yaml"` | File name projected under `/etc/permission-config/url-remapper/`. |

(permission-service-deployment-configuring-the-url-remapper-in-the-helm-chart)=
### Configuring the URL remapper in the Helm Chart

Chart-managed rules (empty `rewriteRulesRef.name`):

```yaml
config:
  urlRemapper:
    enabled: true
    rewriteRules:
      "https://legacy.example.com/": "https://storage.example.com/"
```

Each rule value is either the target prefix string or an object whose `target` field holds the target prefix. The service ignores any other fields in the object, and both forms can appear in the same map. These two rules are equivalent:

```yaml
rewriteRules:
  "omniverse://models/": "https://mybucket.s3.amazonaws.com/models/"
```

```yaml
rewriteRules:
  "omniverse://models/":
    target: "https://mybucket.s3.amazonaws.com/models/"
```

An object without a string `target` field is a configuration error, and the service fails to start.

The chart renders `url-remapper.yaml` into the same ConfigMap as seeded policies and metadata when present, mounts it at `/etc/permission-config/`, and sets `ENABLE_URL_REMAPPER` and `URL_REMAPPER_FILE` on the pod. Upgrades that change the ConfigMap roll the deployment automatically via the chart's config checksum annotation.

To supply rules from an existing ConfigMap, set `rewriteRulesRef.name` and optionally `rewriteRulesRef.key`; the mounted file must use the same `enabled` and `rewriteRules` YAML shape as the chart-managed form.

After changing remapper settings, install or upgrade the release as in {ref}`Install the Permission Service (Config File Mode) <permission-service-deployment-install-the-permission-service-config-file-mode>` or {ref}`Deploy in Postgres Mode <permission-service-deployment-deploy-in-postgres-mode>`.

(permission-service-deployment-ingress-access)=
## Ingress Access

To expose the Permission Service outside the cluster, enable the `httpProxy` section in your values file. The
chart creates [Contour HTTPProxy](https://projectcontour.io/docs/) resources for both REST and gRPC endpoints.

```yaml
httpProxy:
  enabled: true
  fqdn:
    host: "permissions"
    domain: "{DNS_URL}"
```

permission-values.yaml

Redeploy the Permission Service with the updated values:

```bash
helm upgrade --install permission-service . -f permission-values.yaml --namespace storage-apis
```

Verify the HTTPProxy resources were created:

```bash
kubectl get httpproxy -n storage-apis
```

For full details on setting up Contour as a load balancer, configuring DNS records, and enabling TLS, see the
[Ingress Configuration](https://docs.omniverse.nvidia.com/ovstorage/ovstorage-guide/latest/dev-journey/full-deployment/ingress.html)
guide.

(permission-service-deployment-clean-up)=
## Clean Up

To clean up: uninstall the Helm release, delete secrets, and remove the namespace.

```bash
# uninstall the permission service
helm uninstall permission-service -n storage-apis

# delete the secrets
kubectl delete secret permission-service-secret -n storage-apis

# if using CloudNativePG, delete the cluster
kubectl delete cluster permission-pg -n storage-apis

# delete the namespace
kubectl delete namespace storage-apis

# validate the namespace was deleted
kubectl get namespace storage-apis
```

(permission-service-deployment-references)=
## References

- [Cedar Policy Language](https://www.cedarpolicy.com/) — product home for Cedar
- [Cedar documentation](https://docs.cedarpolicy.com/) — language reference and guides
- [CloudNativePG Documentation](https://cloudnative-pg.io/documentation/) — operator for running PostgreSQL in Kubernetes
- [Contour HTTPProxy](https://projectcontour.io/docs/) — ingress resource used by the Helm chart
- [Ingress Configuration](https://docs.omniverse.nvidia.com/ovstorage/ovstorage-guide/latest/dev-journey/full-deployment/ingress.html) — full guide for Contour, DNS, and TLS setup
- [Amazon RDS for PostgreSQL](https://aws.amazon.com/rds/postgresql/) — managed PostgreSQL on AWS
- [Azure Database for PostgreSQL](https://azure.microsoft.com/en-us/products/postgresql/) — managed PostgreSQL on Azure
- [Cloud SQL for PostgreSQL](https://cloud.google.com/sql/docs/postgres) — managed PostgreSQL on GCP

(permission-service-deployment-see-next)=
## See next

Continue with {ref}`Database Configuration <permission-service-database-configuration-database-configuration>` for storage backends, init file formats, and scope inference.

For Omniverse platform deployments, you may also configure the
[Discovery Service](https://docs.omniverse.nvidia.com/ovstorage/ovstorage-guide/latest/dev-journey/full-deployment/discovery-service.html)
so clients resolve every service through a single endpoint.
