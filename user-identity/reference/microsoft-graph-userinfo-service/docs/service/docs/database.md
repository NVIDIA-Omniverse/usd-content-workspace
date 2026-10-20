(userinfo-service-database-database-configuration)=
# Database Configuration

User Info Service uses a PostgreSQL database as the shared store that carries directory
data from the writer to the readers. The writer keeps this store aligned with the identity
provider; readers load it on startup and refresh it periodically. This page describes what
the store holds, how to point the service at it, how to secure the connection, how to supply
static or rotating credentials, and how to size the connection budget when running many
readers.

The store is required for the recommended synchronized-directory deployment. It is **not**
required for the reader-only live mode, in which every request is answered directly from the
identity provider and no database is used. See
{ref}`User Info Service Configuration <userinfo-service-deployment-configuration>` for how to choose between the two
deployments.

(userinfo-service-database-the-shared-store)=
## The Shared Store

A single database is shared by the writer and every reader:

- The **writer** performs the initial full synchronization and then applies only what has
  changed on each cycle, saving the result to the store. Only one writer runs at a time.
- Each **reader** loads the stored directory into memory on startup and reloads it on a fixed
  interval. Readers never write to the store and never contact the identity provider for
  synchronization.

Separating the two roles keeps the synchronization work off the request path: a reader
becomes ready in seconds by loading what the writer has already saved, instead of waiting for
a full synchronization to finish. For how the writer and readers stay aligned and what data
freshness callers can expect, see
{ref}`Directory Synchronization and Data Freshness <userinfo-service-directory-synchronization-directory-synchronization-and-data-freshness>`.

The store holds:

- **User records** — identifier, display name, email, username, given and family name, active
  status, and job title.
- **Group records** — identifier, display name, and description.
- **Group memberships** — which users belong directly to which groups. Only populated when
  membership synchronization is enabled.
- **Synchronization state** — the progress markers the writer uses to resume an interrupted
  synchronization and to fetch only what has changed on the next cycle, which categories of
  data were enabled when the directory was last written, and when the store was last updated.
- **Enterprise-application service accounts** — when the service is scoped to an enterprise
  application, the service accounts assigned to that application are surfaced as users and kept
  separately so they are preserved across incremental cleanup. See
  {ref}`Directory Scoping <userinfo-service-directory-scoping-directory-scoping>`.

The service creates and, where needed, upgrades the structures it requires on startup, so an
empty database is enough to begin. A separate schema migration step is not required.

(userinfo-service-database-connection-settings)=
## Connection Settings

Point the service at the database with the following settings. Each has an environment
variable and a corresponding Helm value under `providers.cache.postgres`.

| Setting | Environment variable | Helm value (`providers.cache.postgres.*`) | Default | Description |
|---|---|---|---|---|
| Enabled | `POSTGRES_ENABLED` | `enabled` | `true` | Use the shared store. Disable only for the reader-only live mode. |
| Host | `POSTGRES_HOST` | `host` | `localhost` | Database server hostname. |
| Port | `POSTGRES_PORT` | `port` | `5432` | Database server port. |
| Database name | `POSTGRES_DBNAME` | `dbname` | `graph-userinfo` | Name of the database used for directory storage. |
| User | `POSTGRES_USER` | `user` | `microsoft_graph_userinfo_service` | Database user the service connects as. |
| Connection budget | `POSTGRES_CONNECTIONS` | `connections` | `4` | Maximum number of simultaneous connections each instance opens. See {ref}`Connection Budget and Scaling <userinfo-service-database-connection-budget-and-scaling>`. |
| Transport security | `POSTGRES_SSL_MODE` | `sslMode` | `prefer` | TLS requirement for the connection. See {ref}`Transport Security <userinfo-service-database-transport-security>`. |

The writer requires the store to be enabled and will refuse to start otherwise. A reader
requires the store whenever caching is enabled; when caching is disabled it runs in live mode
and ignores these settings.

The database user needs permission to read and write the service's data and to create the
structures it uses on first startup. Any PostgreSQL-compatible database can be used. The
{ref}`deployment guide <userinfo-service-deployment-configuration>` recommends Azure Database for PostgreSQL for production
and shows an in-cluster CloudNativePG cluster as an alternative.

```{note}
The defaults in the table above are the service's built-in defaults. The Helm chart sets its
own values in `values.yaml` — notably an empty `host`, which you must supply, and
`sslMode: disable` — so in a chart deployment the effective default comes from the chart, not
from this table.
```

(userinfo-service-database-transport-security)=
## Transport Security

The connection to the database can be encrypted with TLS. The transport-security setting has
three values:

| Value | Behavior |
|---|---|
| `disable` | Never use TLS. The connection is sent in plaintext. |
| `prefer` | Try TLS first, and fall back to plaintext if the server does not accept it. |
| `require` | Require TLS. Refuse to connect if the server does not support it. |

When TLS is negotiated, the database server's certificate is verified against the trusted root
certificates that ship in the service container. Use `require` when the database is reached
over an untrusted network. The deployment examples use `disable` because they connect to a
database on a trusted in-cluster network.

(userinfo-service-database-credentials)=
## Credentials

The service authenticates to the database with a password. The password can be supplied in two
ways: a static password, or a short-lived token that is rotated by an external provider. The
username is always taken from the connection settings above.

(userinfo-service-database-static-password)=
### Static Password

Provide the password directly through the `POSTGRES_PASSWORD` environment variable. In Helm,
reference an existing Kubernetes Secret rather than placing the password in the values file:

```yaml
providers:
  cache:
    postgres:
      password:
        secretRef:
          name: "microsoft-graph-userinfo-service-pg-credentials"
          key: "password"
```

This is the simplest option and is appropriate when the password does not change during the
lifetime of the deployment.

(userinfo-service-database-rotating-token)=
### Rotating Token

For deployments where the database password is short-lived and refreshed periodically, the
service can read the password from a token file instead of a static value. When a token file is
configured, it takes precedence over the static password.

The token file is a small document containing the current credential and the moment it
expires. An external token provider — typically a sidecar running alongside the service — keeps
the file up to date. The service watches the file and, when the credential changes, transparently
begins using the new one for future connections. Connections already open continue to work,
because the database only checks the credential when a connection is first made.

Configure the rotating token in Helm:

```yaml
providers:
  cache:
    postgres:
      tokenFile:
        enabled: true
        path: "/etc/secrets/postgres-token/token"
        pollInterval: "15s"
        waitTimeout: "30s"
        waitInterval: "1s"
        provisionerSidecar:
          container: {}   # the token-provider container to run alongside the service
```

The settings that control this behavior:

| Setting | Environment variable | Helm value (`providers.cache.postgres.tokenFile.*`) | Default | Description |
|---|---|---|---|---|
| Token file | `POSTGRES_TOKEN_FILE` | `path` | `/etc/secrets/postgres-token/token` | Location of the token file inside the container. The chart sets this, and the settings below, only when `tokenFile.enabled` is `true`. |
| Rotation check interval | `POSTGRES_TOKEN_FILE_POLL_INTERVAL` | `pollInterval` | `15s` | How often the file is checked for a rotated credential. |
| Startup wait timeout | `POSTGRES_TOKEN_FILE_WAIT_TIMEOUT` | `waitTimeout` | `30s` | How long to wait at startup for the file to appear before giving up. |
| Startup wait interval | `POSTGRES_TOKEN_FILE_WAIT_INTERVAL` | `waitInterval` | `1s` | How often to re-check for the file while waiting at startup. |

Because the token provider may mount the file a few seconds after the service starts, a missing
or not-yet-written file is retried during the startup wait window rather than treated as an
immediate failure. A credential that is empty or already expired is rejected. The token file
uses the same JSON format as the service-identity access-token file.

(userinfo-service-database-recommended-azure)=
## Recommended: Azure Database for PostgreSQL Flexible Server

For production, run the shared store on
[Azure Database for PostgreSQL flexible server](https://learn.microsoft.com/en-us/azure/postgresql/flexible-server/overview),
Microsoft's fully managed PostgreSQL service. It provides automated backups, high availability,
and scaling without the operational burden of running the database yourself, and it keeps the
directory data in the same cloud as the identity provider the service already reads from.

This managed option pairs naturally with the service's existing Microsoft Entra ID integration.
The same identity system used to read the directory can also authenticate the database
connection, so no separate database password has to be created, stored, or rotated by hand:

- **Authenticate with Microsoft Entra ID.** Enable Microsoft Entra authentication on the server
  and grant access to the identity the service runs as. The service then connects with a
  short-lived Microsoft Entra access token used in place of a password. Set the database user to
  that Entra identity. See
  [Microsoft Entra authentication in Azure Database for PostgreSQL](https://learn.microsoft.com/en-us/azure/postgresql/flexible-server/concepts-azure-ad-authentication)
  and
  [Use Microsoft Entra ID authentication](https://learn.microsoft.com/en-us/azure/postgresql/flexible-server/how-to-configure-sign-in-azure-ad-authentication).
- **Supply the token through the rotating-token credential.** A Microsoft Entra access token is
  valid only for a short period, on the order of an hour, and must be refreshed before it
  expires. Configure the {ref}`rotating token <userinfo-service-database-rotating-token>`
  credential and run a token provider alongside the service that obtains an Entra token and keeps
  the token file current. The service picks up each refreshed token automatically and existing
  connections keep working.
- **Require an encrypted connection.** Azure Database for PostgreSQL enforces encrypted
  connections and rejects unencrypted ones, so set the
  {ref}`transport security <userinfo-service-database-transport-security>` setting to `require`.
  The service verifies the server's certificate against the trusted root certificates in its
  container, which include the authorities Azure uses. See
  [Transport Layer Security in Azure Database for PostgreSQL](https://learn.microsoft.com/en-us/azure/postgresql/flexible-server/security-tls).

Static-password authentication against the managed server also works and is a valid starting
point, but Entra-based authentication with the rotating-token credential is preferred because it
avoids a long-lived database password entirely.

(userinfo-service-database-connection-budget-and-scaling)=
## Connection Budget and Scaling

Each service instance — the writer and every reader — opens up to its configured connection
budget (`connections`, default `4`) against the database. The database must allow at least the
sum of the budgets of all running instances, plus headroom for administrative access.

As a starting point, size the database's connection limit for:

```text
(number of readers + 1 writer) × connection budget per instance + headroom
```

For example, with the default budget of `4` and one writer plus three readers, the service can
use up to sixteen connections. When scaling readers out for more capacity, or when raising the
per-instance budget for higher throughput, raise the database's connection limit to match.

Two points to keep in mind when sizing:

- **Writer full synchronization.** While the writer is performing a full synchronization it
  holds one of its connections aside to coordinate with any other writer, so that only one full
  synchronization runs at a time. Keep the writer's budget at two or more so it retains a
  working connection during a full synchronization.
- **Readers are read-only.** Readers reload the stored directory on their refresh interval and
  otherwise leave their connections idle, so their steady-state load on the database is light.
  Most connection pressure comes from adding more reader replicas rather than from request
  volume.

The number of connections currently in use by each instance is reported as a metric; see
{ref}`Observability <userinfo-service-observability-observability>` for how to monitor it.

(userinfo-service-database-references)=
## References

- [What is Azure Database for PostgreSQL flexible server?](https://learn.microsoft.com/en-us/azure/postgresql/flexible-server/overview)
- [Microsoft Entra authentication in Azure Database for PostgreSQL flexible server](https://learn.microsoft.com/en-us/azure/postgresql/flexible-server/concepts-azure-ad-authentication)
- [Use Microsoft Entra ID authentication in Azure Database for PostgreSQL flexible server](https://learn.microsoft.com/en-us/azure/postgresql/flexible-server/how-to-configure-sign-in-azure-ad-authentication)
- [Transport Layer Security (TLS) in Azure Database for PostgreSQL flexible server](https://learn.microsoft.com/en-us/azure/postgresql/flexible-server/security-tls)

(userinfo-service-database-see-next)=
## See next

- {ref}`Directory Synchronization and Data Freshness <userinfo-service-directory-synchronization-directory-synchronization-and-data-freshness>`
