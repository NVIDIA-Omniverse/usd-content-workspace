(permission-service-database-configuration-database-configuration)=
# Database Configuration

The Permission Service supports two storage backends: **PostgreSQL** (default) and **Config File** (read-only YAML
files).

(permission-service-database-configuration-selecting-the-database-type)=
## Selecting the Database Type

Set the `DATABASE_TYPE` environment variable or `--database-type` CLI argument:

| Value         | Description                                                               |
|---------------|---------------------------------------------------------------------------|
| `postgres`    | Uses PostgreSQL as the read-write storage backend (default).              |
| `config-file` | Uses YAML files as a read-only storage backend; writes are not supported. |

(permission-service-database-configuration-helm-chart)=
### Helm Chart

All storage-related settings are under the `database` section in Helm values:

```yaml
database:
  type: "config-file"       # or "postgres"

  # Initialization data for policies and metadata.
  # The chart creates a ConfigMap and mounts these as files inside the container.
  # In config-file mode, these files are used as the storage backend.
  # In postgres mode, policies are loaded as system policies on startup.
  init:
    policies:
      - policy: "permit(principal, action, resource);"
    services:
      - name: "my-service"

  # Postgres settings (ignored when type is "config-file")
  postgres:
    host: "postgres-host"
    port: 5432
    user: "postgres"
    # ... other postgres settings
```

When `database.init.policies` or `database.init.services` are provided, the chart creates a ConfigMap and mounts it at
`/etc/permission-config/`. The `INIT_POLICIES_FILE` and `INIT_METADATA_FILE` env vars are set to the mounted file paths.

In **config-file** mode, these files are used as the read-only storage backend. The `database.postgres` settings are
ignored.

In **postgres** mode, policies from `database.init.policies` are loaded as system policies on startup.

(permission-service-database-configuration-environment-variables)=
### Environment Variables

| Variable             | Description                            |
|----------------------|----------------------------------------|
| `DATABASE_TYPE`      | `postgres` or `config-file`            |
| `INIT_POLICIES_FILE` | Path to the policy YAML file on disk   |
| `INIT_METADATA_FILE` | Path to the metadata YAML file on disk |

(permission-service-database-configuration-config-file-mode)=
## Config File Mode

In config file mode, the service reads policies and metadata from YAML files on disk. Both files are automatically
reloaded when their modification time changes — no service restart is required.

Every request that would create, update, or delete a policy or service metadata record is rejected with
`501 Not Implemented` and a "not supported" message.

(permission-service-database-configuration-policy-file-format)=
### Policy File Format

```yaml
policies:
  - policy: "<cedar_policy_text>"
```

Each entry under `policies` must include a `policy` field with Cedar policy text. Scope fields are inferred from Cedar
text for both database backends.

The same inference behavior is used by REST write APIs (`PUT /v1beta/policies/` and
`PUT /v1beta/policies/batch/`): each payload item must contain exactly one Cedar statement. Client-provided
`principal`/`action`/`resource` fields are ignored when present.

(permission-service-database-configuration-scope-inference)=
### Scope Inference

Every policy stored in the service has five optional scopes: **principal**, **group**, **action**, **resource**, and
**resource prefix**.
When an authorization request arrives, the service fetches all policies whose scopes match the request — a policy is
fetched when its scope equals the corresponding request field, or when its scope is not set. A policy with an unset
scope is called a **global policy** for that scope: it is fetched regardless of the value provided in the
authorization request.

The **group** scope is matched against the requesting principal's group memberships rather than a single request
field: a group-scoped policy is fetched when the request principal belongs to that group (its id appears in the
configured groups claim — see {ref}`Groups and membership <permission-service-policy-language-groups-and-membership>`),
in addition to the usual unscoped policies.

The **resource prefix** scope is matched against the folder hierarchy of the requested resource: a prefix-scoped policy
is fetched when its folder is one of the folders that contain the request's resource id (see
{ref}`Resource hierarchies <permission-service-policy-language-resource-hierarchies>`).

After fetching, the Cedar policy text is evaluated against the full request to produce the final allow or deny decision.
Scopes control which policies are fetched; Cedar controls the outcome. For how fetched policies are grouped by `order` and combined under each resource type's evaluation priority, see {ref}`How authorization requests are evaluated <permission-service-policy-evaluation-how-authorization-requests-are-evaluated>`.

When a policy is loaded from the configuration file, the service reads the Cedar text and infers each scope
automatically:

- If the Cedar constraint is an exact equality, the scope is set to that value.
  For example, `principal == Principal::"alice"` sets the principal scope to `alice`, and
  `action == Action::"storage:read"` sets the action scope to `storage:read`.
- If the principal constraint is membership in a `Group` entity, the **group** scope is set to that group id. For
  example, `principal in Group::"platform-admins"` sets the group scope to `platform-admins` (and leaves the principal
  scope unset). This is the first-class, persisted form of `principal.groups.contains("platform-admins")`.
- If the resource constraint is a hierarchy membership, `resource in <Type>::"<url>/"`, the **resource prefix** scope
  is set to that folder (and the resource scope is left unset). For example,
  `resource in folder::"https://main/docs/"` sets the resource prefix scope to the `folder` subtree
  `https://main/docs/`.
- If the Cedar constraint cannot be reduced to a single exact value (for example `is`, `principal in` a non-`Group`
  type, lists with multiple values, or an unconstrained `principal`/`action`/`resource`), the scope is left unset and
  the policy becomes global for that scope.

A policy where all scopes are unset — such as `permit(principal, action, resource);` — is fully global. It is
fetched for every authorization request, and Cedar alone decides whether to allow or deny.

Each policy entry must contain exactly one Cedar policy statement. If an entry contains multiple statements, policy
loading fails.

(permission-service-database-configuration-limitations)=
### Limitations

- Scopes are used for policy retrieval only. Cedar policy text is always authoritative for the final authorization
  decision.
- Resource IDs inferred from Cedar follow the same normalization and encoding behavior used by policy validation.

(permission-service-database-configuration-policy-example)=
#### Policy Example

```yaml
policies:
  # Global permit — no constraints, applies to everything
  - policy: "permit(principal, action, resource);"

  # Scoped to a specific action
  - policy: 'permit(principal, action == Action::"my-service:read", resource);'

  # Scoped to action + principal + resource
  - policy: >-
      permit(
        principal == User::"user123",
        action == Action::"my-service:write",
        resource == Resource::"my-service:document/doc1"
      );

  # Cedar policy with a "when" condition
  - policy: >-
      permit(principal, action, resource)
      when { principal.sub == "admin" && resource.tag == "public" };

  # Cedar policy with an "unless" condition — no constraints (global)
  - policy: >-
      forbid(principal, action, resource)
      unless { principal.sub == "superadmin" };
```

(permission-service-database-configuration-metadata-file-format)=
### Metadata File Format

```yaml
services:
  - name: "<service_name>"
    principal:
      idClaim: "<claim>"                     # optional, JWT claim used as principal ID
    actions: # optional, list of action names
      - "<action_name>"
    resourceTypes: # optional, list of resource types
      - type: "<resource_type>"
        evaluationPriority: "<permit|forbid>"   # optional, defaults to "forbid"
```

(permission-service-database-configuration-metadata-example)=
#### Metadata Example

```yaml
services:
  - name: "my-service"
    principal:
      idClaim: "sub"
    actions:
      - "read"
      - "write"
    resourceTypes:
      - type: "document"
        evaluationPriority: "forbid"
```

(permission-service-database-configuration-see-next)=
## See next

Continue with {ref}`Authenticating with an OpenID Connect Identity Provider <permission-service-authentication-authenticating-with-an-openid-connect-identity-provider>` for inbound bearer-token validation and outbound service identity.
