(permission-service-policy-language-writing-cedar-policies-for-permission-service)=
# Writing Cedar policies for Permission Service

Permission Service evaluates access using the [Cedar](https://www.cedarpolicy.com/) policy language. Policies are plain Cedar text: each stored policy is **exactly one** Cedar statement (`permit` or `forbid`). You add them through Helm `database.init.policies`, a config-file policy YAML, or the REST policy management API (see {ref}`Database configuration <permission-service-database-configuration-database-configuration>` and {ref}`Deployment <permission-service-deployment-permission-service-configuration>`).

This page is a **practical introduction** for authors who are new to Cedar syntax. For the full language specification, grammar, and advanced topics, see the **[official Cedar documentation](https://docs.cedarpolicy.com/)**.

(permission-service-policy-language-cedar-syntax-overview)=
## Cedar syntax overview

A Cedar policy says who may or may not do what, optionally refined by extra conditions.

- **`permit`** — if the policy applies and its conditions hold, the request may be allowed (subject to other policies).
- **`forbid`** — if the policy applies and its conditions hold, the request is denied. Forbid overrides permit by default when both match.

The core shape is:

```text
permit(
  <principal constraint>,
  <action constraint>,
  <resource constraint>
);
```

or the same with `forbid`. Each constraint can be:

- The bare name `principal`, `action`, or `resource` — meaning “any” for that slot in the policy head.
- An equality such as `principal == Principal::"alice"` — only that entity.
- Other Cedar constraints (`in`, `is`, lists, and so on).

You can append Boolean conditions:

- **`when { ... }`** — the policy applies only when the expression is true.
- **`unless { ... }`** — the policy does **not** apply when the expression is true.

**Examples (patterns used with this service):**

```text
// Allow one principal, one action, one resource (all three scoped by equality)
permit(
  principal == Principal::"alice",
  action == Action::"my-service:read",
  resource == document::"doc-1"
);
```

```text
// Allow any principal for one action; narrow further in "when"
permit(
  principal,
  action == Action::"my-service:read",
  resource
)
when { resource.id like "/public/*" };
```

```text
// Deny by default path, except for a break-glass principal attribute
forbid(principal, action, resource)
unless { principal.breakglass == true };
```

Cedar has more operators and types (sets, records, `in`, `is`, and so on). Use the [official Cedar documentation](https://docs.cedarpolicy.com/) for complete syntax and semantics.

(permission-service-policy-language-what-principal-action-and-resource-refer-to-in-a-policy)=
## What `principal`, `action`, and `resource` refer to in a policy

A policy is evaluated against a single authorization request, and inside the policy the names `principal`, `action`, and `resource` refer to Cedar entities built from that request. Writing useful policies requires knowing what those entities look like in this service.

(permission-service-policy-language-principal)=
### Principal

The **principal entity id** is **not** hard-coded to the JWT `sub` claim. For each request the service picks the id from the caller's claims in this order:

1. The **`idClaim`** configured in the service metadata for the target action (per-service setting; see {ref}`Database configuration <permission-service-database-configuration-metadata-file-format>`).
2. The deployment-wide principal id claim set by the Helm value **`openId.principalIdClaim`**, which defaults to **`sub`**.
3. The standard **`sub`** claim, as a final fallback.

The first of these that is present in the caller's claims becomes `Principal::"<that value>"` for the request. Different services in the same deployment can therefore identify the same user by different ids (for example, `email` for one service and `sub` for another).

The principal entity also exposes attributes Cedar conditions can read:

- **`principal.sub`** is always set to the principal id used for this request (whatever claim was selected above).
- All other fields from the caller's claims / principal `info` are flattened onto the entity, so expressions like `principal.groups`, `principal.email`, or `principal.department.name` work when those claims are present.

(permission-service-policy-language-action)=
### Action

The **action entity** is built from the `action` in the request:

- Type: **`Action`**.
- Id: **`"<service>:<name>"`** (for example `Action::"storage:read"`).
- No attributes; policies match actions in the policy head, not via `action.*`.

(permission-service-policy-language-resource)=
### Resource

When the request includes a resource, the service builds a resource entity:

- Type: the **`type`** from the request (for example `File`, `document`).
- Id: the **`id`** from the request.
- Attributes: **`id`** and **`type`** are always set (as strings), and any fields in the resource's `data` object are flattened onto the entity — for example `resource.path` or `resource.owner` when those fields are sent.

Resource ids are normalized/encoded consistently in policies and requests (see {ref}`Database configuration <permission-service-database-configuration-scope-inference>`); use the same string form your clients send on the API.

If a request has no resource, policies must not constrain `resource` by equality or reference `resource.*` in conditions.

(permission-service-policy-language-actions-and-resources-for-omniverse-services)=
## Actions and resources for Omniverse services

Actions and resource types available in policies are defined by each deployment's service metadata. The tables below list the actions and resource types registered for the standard Omniverse services in a reference (development) deployment; your environment may register a different set.

Use these values directly in the policy head — `Action::"<service>:<name>"` for actions and `<resource type>::"<id>"` for resources.

(permission-service-policy-language-storage-service)=
### `storage-service`

Actions:

| Name | Cedar reference | APIs |
|------|-----------------|------|
| `read` | `Action::"storage-service:read"` | Read APIs on `object` and `folder` — retrieving object content, listing folder contents, and reading object/folder metadata. |
| `write` | `Action::"storage-service:write"` | Mutating APIs on `object` and `folder` — creating, updating, moving, and deleting objects and folders. |

Resource types:

| Type | Cedar reference |
|------|-----------------|
| `object` | `object::"<id>"` |
| `folder` | `folder::"<id>"` |

Example:

```text
permit(
  principal,
  action == Action::"storage-service:read",
  resource
)
when { resource is object || resource is folder };
```

Because `object` and `folder` ids are URLs, a policy can also grant access to a whole subtree at once with a hierarchy `in` — here every `object` and `folder` under `https://main/docs/`:

```text
permit(
  principal,
  action == Action::"storage-service:read",
  resource in folder::"https://main/docs/"
);
```

See {ref}`Resource hierarchies <permission-service-policy-language-resource-hierarchies>` for how these subtree matches work and how the `folder` parent type is configured.

(permission-service-policy-language-event-consumer-service)=
### `event-consumer-service`

This service registers actions only; requests target no resource.

Actions:

| Name | Cedar reference | APIs |
|------|-----------------|------|
| `consume-all-storage-create-events` | `Action::"event-consumer-service:consume-all-storage-create-events"` | APIs that subscribe to or read the global stream of storage "object/folder created" events. |
| `consume-all-storage-delete-events` | `Action::"event-consumer-service:consume-all-storage-delete-events"` | APIs that subscribe to or read the global stream of storage "object/folder deleted" events. |
| `consume-durable-queues` | `Action::"event-consumer-service:consume-durable-queues"` | APIs that read and acknowledge messages from durable event queues. |
| `create-durable-queues` | `Action::"event-consumer-service:create-durable-queues"` | APIs that provision new durable event queues. |
| `delete-durable-queues` | `Action::"event-consumer-service:delete-durable-queues"` | APIs that remove durable event queues. |
| `inspect-and-replay-all-dlq` | `Action::"event-consumer-service:inspect-and-replay-all-dlq"` | APIs that inspect and replay messages across all dead-letter queues (DLQs). |

Resource types: none.

Because no resource is attached to these requests, write policies with an unconstrained `resource` slot and do not reference `resource.*` in conditions:

```text
permit(
  principal in Group::"event-consumers",
  action in [
    Action::"event-consumer-service:consume-durable-queues",
    Action::"event-consumer-service:create-durable-queues",
    Action::"event-consumer-service:delete-durable-queues"
  ],
  resource
);
```

When Permission Service runs its own built-in cross-replica cache-invalidation consumer (`notifications.consumer.enabled: true` — see {ref}`Consuming change events for cross-replica cache invalidation <permission-service-notifications-cross-replica-cache-invalidation>`), and the guarding deployment enforces authorization on the Consumer API, the Permission Service principal itself needs a permit for the `event-consumer-service` consume action, exactly like the `event-aggregation-service:publish-event` permit granted for publishing:

```text
permit(
  principal == Principal::"permission-service",
  action == Action::"event-consumer-service:consume-durable-queues",
  resource
);
```

(permission-service-policy-language-event-aggregation-service)=
### `event-aggregation-service`

Actions:

| Name | Cedar reference | APIs |
|------|-----------------|------|
| `publish-event` | `Action::"event-aggregation-service:publish-event"` | APIs that publish an event of a given `EventType` to the aggregation service. |

Resource types:

| Type | Cedar reference |
|------|-----------------|
| `EventType` | `EventType::"<id>"` |

Example:

```text
permit(
  principal,
  action == Action::"event-aggregation-service:publish-event",
  resource == EventType::"storage.object.created"
);
```

(permission-service-policy-language-userinfo)=
### `userinfo`

Actions:

| Name | Cedar reference | APIs |
|------|-----------------|------|
| `list-users` | `Action::"userinfo:list-users"` | API that lists `User` records. |
| `get-user` | `Action::"userinfo:get-user"` | API that reads a single `User` record. |
| `list-user-groups` | `Action::"userinfo:list-user-groups"` | API that lists the `Group`s a given user belongs to. |
| `get-user-group` | `Action::"userinfo:get-user-group"` | API that reads a single user-to-group membership entry. |
| `list-groups` | `Action::"userinfo:list-groups"` | API that lists `Group` records. |
| `get-group` | `Action::"userinfo:get-group"` | API that reads a single `Group` record. |
| `list-group-members` | `Action::"userinfo:list-group-members"` | API that lists the members (`User`s) of a given group. |
| `get-group-member` | `Action::"userinfo:get-group-member"` | API that reads a single group-member entry. |

Resource types:

| Type | Cedar reference |
|------|-----------------|
| `User` | `User::"<id>"` |
| `Group` | `Group::"<id>"` |

The `User` and `Group` resource types here are the **resources being queried** by `userinfo` actions — they are not Cedar group-membership entities. See {ref}`Groups and membership <permission-service-policy-language-groups-and-membership>` for how to model role or group checks on the principal.

Example:

```text
permit(
  principal,
  action in [
    Action::"userinfo:list-users",
    Action::"userinfo:get-user"
  ],
  resource
)
when { resource is User };
```

(permission-service-policy-language-what-policies-you-can-add)=
## What policies you can add

You can add **permit** policies (allow paths) and **forbid** policies (explicit deny). Together they express your authorization model; Cedar’s default for a request is deny if no permit applies, and **forbid** can override **permit** when both match.

Policies may:

- Constrain **principal**, **action**, and/or **resource** in the policy head (equality, wildcards via unconstrained slots, `when` / `unless`, and so on).
- Use **`when`** and **`unless`** on attributes of `principal`, `resource`, and on **context** passed in the authorization request (see below).

Operational rules for this service:

- Each policy record must contain **exactly one** Cedar statement. Multiple statements in one stored policy are rejected.
- Policies are **not** validated against the registered service **metadata**: a policy may reference actions and resource types that are not registered. Metadata is used at request time — the service's `idClaim`, and the resource type's `evaluationPriority` and parent type — so policies should still align with the actions and resource types your deployment registers.

(permission-service-policy-language-policy-scopes-principal-action-resource)=
## Policy scopes (principal, action, resource)

The service attaches optional **scopes** to each stored policy: a **principal** or **group** scope, an **action** scope, and a **resource** or **resource-prefix** scope. Scopes are used to **retrieve** relevant policies efficiently: on each authorization request, policies whose scopes match (or are unset for that dimension) are candidates for evaluation.

How scopes are **inferred** from the Cedar text (summary):

- **Principal scope** is set only when the head uses **equality** on the principal: `principal == Principal::"<id>"`.
- **Group scope** is set when the head uses **`principal in Group::"<id>"`** (see {ref}`Groups and membership <permission-service-policy-language-groups-and-membership>`). Other principal forms (`principal` alone, `in` on a non-`Group` type, `is`, and so on) leave both principal and group scope unset.
- **Action scope** is set for **`action == Action::"<service>:<name>"`**, or for **`action in [Action::"svc:n"]`** with **exactly one** action in the set. Otherwise action scope is unset.
- **Resource scope** is set when the head pins the resource with **equality** — `resource == <Type>::"<id>"`, where `<Type>` is the resource kind — and scopes the policy to one resource.
- **Resource-prefix scope** is set when the head uses a **hierarchy** `in` on a URL folder — `resource in <Type>::"<url>/"` — and scopes the policy to a whole URL subtree (see {ref}`Resource hierarchies <permission-service-policy-language-resource-hierarchies>`). Other resource forms (`resource` alone, `is`, and so on) leave both resource and resource-prefix scope unset.

If a scope is unset for a dimension, any request value on that dimension can still match this policy for retrieval purposes. The full Cedar text (including `when` / `unless`) still determines the final allow/deny decision.

(permission-service-policy-language-global-and-unscoped-policies)=
## Global and unscoped policies

A policy is **global** along a dimension when the service **does not** infer a scope for that dimension from the policy head. That happens when the Cedar head does **not** pin that slot via the scope patterns above (for example you write unconstrained `principal`, `action`, or `resource`, or you use `is` / multiple actions, and so on).

Two `in` forms are **not** global: `principal in Group::"<id>"` sets a group scope (see {ref}`Groups and membership <permission-service-policy-language-groups-and-membership>`) and `resource in <Type>::"<url>/"` sets a resource hierarchy scope (see {ref}`Resource hierarchies <permission-service-policy-language-resource-hierarchies>`).

A policy that leaves **all** scopes unset is **fully global**: it is a candidate for **every** authorization request. A common example is:

```text
permit(principal, action, resource);
```

Fully global policies are useful for “open” test environments; in production you usually combine narrower scoped policies with explicit `forbid` rules and `when` / `unless` conditions.

(permission-service-policy-language-when-and-unless-conditions)=
## `when` and `unless` conditions

Use **`when { expression }`** to require extra facts for the policy to apply. Use **`unless { expression }`** to **suppress** the policy when the expression is true.

- **`when`** — if the head matches the request’s entities, the policy contributes allow/deny only if `when` evaluates to true (and any `unless` does not block it).
- **`unless`** — if the expression is true, the policy does not apply for that request.

You can combine them on the same policy. Expressions may reference:

- **`principal.*`** — including `sub` and fields from principal `info`.
- **`resource.*`** — including `id`, `type`, and fields from resource `data`.
- **Context** — attributes from the optional **context** object on the authorization request, if your deployment supplies schema-valid context Cedar can see (see Cedar’s context documentation in the [official guide](https://docs.cedarpolicy.com/)).

**Examples:**

```text
permit(principal, action == Action::"docs:read", resource)
when { principal.sub like "svc-*" && resource.type == "document" };
```

```text
forbid(principal, action, resource)
when { context.ipRange == "10.0.0.0/8" }
unless { principal.mfa == true };
```

(permission-service-policy-language-groups-and-membership)=
## Groups and membership

Group membership is a **first-class** policy scope. A policy head may target a group directly:

```text
permit(principal in Group::"platform-admins", action, resource);
```

This is **exactly equivalent** to the membership condition below, and the two forms evaluate identically:

```text
permit(principal, action, resource)
when { principal.groups.contains("platform-admins") };
```

At evaluation time the service builds a `Group::"<id>"` entity for every id in the principal's groups claim and attaches them as **parents** of the principal entity, so Cedar evaluates `principal in Group::"<id>"` natively. The first-class form is preferred because the group scope is structured and client-parseable: it is inferred from the policy head, persisted, exposed in `GET`/list/diagnostics responses, and usable as a filter (`GET /v1beta/policies/?group=<id>`).

The claim used for membership is configurable and defaults to `groups` (see {ref}`Helm values <permission-service-authentication-helm-values>` — `openId.principalGroupsClaim` / `PRINCIPAL_GROUPS_CLAIM`). It must be a JSON array of group id strings. Populate it (or your configured claim) in the principal payload your services send to Permission Service so Cedar can resolve the memberships.

```{note}
The `Group::"<id>"` scope here models **principal group membership**. It is distinct from the `Group` *resource* type queried by the `userinfo` service ({ref}`userinfo <permission-service-policy-language-userinfo>`), which represents group records being acted upon rather than the caller's memberships.
```

Membership conditions on `principal.groups` remain fully supported and can be combined with other `when`/`unless` clauses; use them when you need richer expressions than a single group scope.

Just as `principal in Group::"<id>"` targets a principal by membership, `resource in <Type>::"<url>/"` targets a resource by its place in a URL hierarchy; see {ref}`Resource hierarchies <permission-service-policy-language-resource-hierarchies>`.

(permission-service-policy-language-resource-hierarchies)=
## Resource hierarchies

For resource types whose ids are URLs — for example storage `object` and `folder` ids such as `https://main/docs/report.pdf` — a policy head can target a whole subtree instead of a single resource:

```text
permit(
  principal,
  action == Action::"storage-service:read",
  resource in folder::"https://main/docs/"
);
```

`resource in <Type>::"<url>/"` matches the folder node itself **and every resource nested under it** — `https://main/docs/report.pdf`, `https://main/docs/reports/q1.pdf`, and so on. The subtree is derived from the URL path: each `/`-separated segment is a level in the hierarchy, and the scope covers all descendants of the given folder.

How these policies match:

- The id after `in` must be a **valid absolute URL** and is normalized to a trailing `/` (the folder boundary), so `resource in folder::"https://main/docs"` and `resource in folder::"https://main/docs/"` mean the same subtree.
- Matching is by **folder boundary**, not raw string prefix: `resource in folder::"https://main/docs/"` matches `https://main/docs/report.pdf` but not a sibling such as `https://main/docs-archive/report.pdf`.
- A request whose resource id is itself a folder form (ends in `/`) matches **inclusively** — a policy scoped to `folder::"https://main/docs/"` applies to a request on exactly `https://main/docs/`.
- `<Type>` is the **parent type** of the resources you want to match (see below), not necessarily the request resource's own type.

(permission-service-policy-language-configuring-parent-relationships)=
### Configuring parent relationships

The `<Type>` written in a `resource in <Type>::"<url>/"` head is the resource type's **parent type**. Each registered resource type may declare a parent type in its service metadata; when a request targets a resource of that type, the service treats the URL folders above it as entities of that parent type, so `resource in <parentType>::"<url>/"` matches those containing folders natively.

For example, storage `object` resources declare `folder` as their parent type. A policy scoped to `folder::"<url>/"` therefore matches both `object`s and `folder`s nested under that URL. When no parent type is configured, a resource's folders are typed with the resource's **own** type instead.

Parent type is configured alongside the resource type in service metadata; see {ref}`Parent type for resource hierarchies <permission-service-metadata-parent-type-for-resource-hierarchies>`.

(permission-service-policy-language-meta-permissions)=
## Meta-permissions: governing Permission Service itself

Permission Service protects its own management, metadata, and diagnostics APIs with a reserved service named **`permissions`**. You write policies for its actions exactly like any other service — `permit`/`forbid` with the same heads, scopes, and `when`/`unless` conditions — but the entities these policies see are built from the **management operation** being attempted, not from a downstream service request.

| Action | Gates | Resource in the policy head |
|--------|-------|-----------------------------|
| `permissions:view` | Reading policies (`GET /v1beta/policies/`) and authorizing a request whose `principal` differs from the caller. | The **resource being probed** in the authorization request. |
| `permissions:edit` | Creating and deleting policies (`PUT /v1beta/policies/`, `PUT /v1beta/policies/batch/`, and `DELETE /v1beta/policies/{id}`). | The **resource governed by the policy** being created or deleted. |
| `permissions:meta:read` | Reading service metadata (`GET /v1beta/services/`). | None. |
| `permissions:meta:write` | Writing service metadata (`PUT`/`DELETE /v1beta/services/`). | None. |
| `permissions:diagnostics` | Calling `POST /v1beta/diagnostics/authorize/`. | None. |

Meta-permissions are only evaluated when authentication is enabled. For seeding broad administrative access on first deployment, see {ref}`Granting administrative permissions <permission-service-deployment-granting-administrative-permissions>`; this section covers how to **scope** those grants.

(permission-service-policy-language-meta-permissions-simple-grants)=
### Simple grants

The most common meta-permission policies grant an action to a principal or group with an unconstrained `resource`, which matches every management operation:

```text
// Anyone in the "permission-admins" group may create and delete any policy
permit(
  principal in Group::"permission-admins",
  action == Action::"permissions:edit",
  resource
);
```

The non-resource actions (`permissions:meta:read`, `permissions:meta:write`, `permissions:diagnostics`) are **always** written with an unconstrained `resource`, because those operations carry no resource — do not pin `resource` by equality or reference `resource.*` for them:

```text
permit(
  principal in Group::"permission-admins",
  action == Action::"permissions:meta:write",
  resource
);
```

(permission-service-policy-language-meta-permissions-scoping-by-resource)=
### Scoping `permissions:edit` and `permissions:view` by resource

`permissions:edit` and `permissions:view` are evaluated against the **real target resource**, so you can scope them with the same resource heads used for ordinary policies — equality on a single resource or a hierarchy `in` on a URL subtree (see {ref}`Resource hierarchies <permission-service-policy-language-resource-hierarchies>`):

```text
// Editors may only manage policies for resources under a specific folder subtree
permit(
  principal in Group::"docs-editors",
  action == Action::"permissions:edit",
  resource in folder::"https://main/docs/"
);
```

```text
// Explicitly deny editing permissions for one protected subtree, for everyone
forbid(
  principal,
  action == Action::"permissions:edit",
  resource in folder::"https://main/docs/secret/"
);
```

- For **`permissions:edit`**, the resource head is matched against the resource that the created/deleted policy governs (its inferred `resource == …` or `resource in …` scope). A policy that governs no resource (its head leaves the resource slot unset) presents **no** resource to the meta-check, so a resource-scoped meta-policy will not match it — grant those cases with an unconstrained `resource` meta-policy instead.
- For **`permissions:view`**, the resource head is matched against the resource named in the authorization request being probed.
- Write the resource id in the same canonical form your clients use. If the deployment configures a {ref}`URL remapper <permission-service-deployment-url-remapper>`, resource ids in both the governed policy and the meta-policy head are remapped to their canonical URL **before** the check, so scope meta-policies with the canonical (post-remap) URL.

(permission-service-policy-language-meta-permissions-context)=
### Refining with `context`

The management operation's other details are supplied to the meta-check through the Cedar **`context`**, so `when`/`unless` conditions can narrow a grant without pinning the resource. The service populates:

| Field | `permissions:edit` | `permissions:view` |
|-------|--------------------|--------------------|
| `context.principal.id` | Principal the edited policy targets (if it pins one). | Principal whose permissions are being probed. |
| `context.action.name`, `context.action.service` | Action the edited policy targets (if it pins one). | Action being probed. |
| `context.group.id` | Group the edited policy targets (if it pins one). | — |

```text
// Editors may only manage policies for the "storage-service"
permit(
  principal in Group::"service-admins",
  action == Action::"permissions:edit",
  resource
)
when { context.action.service == "storage-service" };
```

```text
// An auditor may view only one principal's effective permissions
permit(
  principal == Principal::"auditor",
  action == Action::"permissions:view",
  resource
)
when { context.principal.id == "alice@example.test" };
```

When a scope is unset on the operation, its `context` object is present but empty and the nested attribute is absent — for example, editing a policy that does not pin an action leaves `context.action` as `{}`, so `context.action.service` does not exist. Guard against that with Cedar's `has` operator when a policy might see either shape:

```text
permit(principal in Group::"service-admins", action == Action::"permissions:edit", resource)
when {
  context.action has service &&
  context.action.service == "storage-service"
};
```

(permission-service-policy-language-see-next)=
## See next

- {ref}`Managing policies through the REST API <permission-service-policy-management-managing-policies-through-the-rest-api>` — storing, listing, and removing the policies described here while the service runs.

Related pages:

- [Official Cedar documentation](https://docs.cedarpolicy.com/) — language reference, schema, and best practices
- {ref}`How authorization requests are evaluated <permission-service-policy-evaluation-how-authorization-requests-are-evaluated>` — how policies, scopes, `order`, and resource evaluation priority combine at request time
- {ref}`Database configuration <permission-service-database-configuration-database-configuration>` — policy file format, scope inference details, and examples
- {ref}`Deployment <permission-service-deployment-permission-service-configuration>` — configuring initial policies in Helm
