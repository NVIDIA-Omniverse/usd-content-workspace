# Least-privilege Cedar policies for the stack

> **Applies to:** Permission Service policies and service metadata for Storage, notifications, User Info, the Permission UI, and your own services · **Released:** permission-service 0.0.53 · **This repo's snapshot:** permission-service 0.0.54 (policy language, evaluation, and meta-permissions unchanged from 0.0.53; the 0.0.54 service docs correct several descriptions, reflected below) · **Sources:** `permissions/reference/permission-service/docs/service/docs/{policy-language,policy-evaluation,deployment,metadata,notifications}.md`, `helm/permission-service/values.yaml` (`database.init.*`), `README.md` (meta-permissions), `user-identity/.../docs/service/docs/authorization.md`

How the service turns a request into a decision is in [`policy-evaluation.md`](../../../../../permissions/reference/permission-service/docs/service/docs/policy-evaluation.md); syntax is in [`policy-language.md`](../../../../../permissions/reference/permission-service/docs/service/docs/policy-language.md). This page gives policies you can seed, and the traps that turn a reasonable-looking policy set into an over-grant.

## The parts of a policy

| Slot | Form in this service | Notes |
|---|---|---|
| Principal | `Principal::"<id>"`, or `principal in Group::"<group-id>"` | `<id>` is the value of the service's `idClaim`, else `openId.principalIdClaim` (`oid` on Entra), else `sub`. Group ids come from the `groups` claim (`openId.principalGroupsClaim`). |
| Action | `Action::"<service>:<name>"` | `<service>` and `<name>` as registered in metadata |
| Resource | `<type>::"<id>"`, or `resource in <parentType>::"<url>/"` | URL subtree match by folder boundary; trailing `/` normalized |
| Conditions | `when { … }`, `unless { … }` | `principal.*` = token claims; `resource.*` = `id`, `type`, and request `data`; `context.*` |
| `order` | integer per policy (`database.init.policies[].order`, API `order`) | Default `config.defaultPolicyOrder` 0; lower runs first; the first order group with any match decides |

Each stored policy holds exactly one Cedar statement. Write principal ids and resource URLs in the canonical form your clients send (after the [URL remapper](../../../../../permissions/reference/permission-service/docs/service/docs/deployment.md#url-remapper), if enabled). From 0.0.54 the remapper resolves `.` and `..` segments before matching, so policies written against the canonical path apply to dot-segment variants too; on 0.0.53 they may not.

## Traps

1. **The fully open policy.** `permit(principal, action, resource);` also grants `permissions:edit`, `permissions:meta:write`, and the rest to every authenticated caller. Never seed it outside an isolated test cluster.
2. **Evaluation priority `permit` in the default seed.** The chart's seed registers storage `object` and `folder` with `evaluationPriority: "permit"`. Inside one order group, any matching `permit` beats a matching `forbid` for those types. A deny you add next to broad permits is ignored. Either give forbids a lower `order` than permits (they are then evaluated first and decide on match), or change those resource types to `forbid` in your `database.init.services` / metadata API.
3. **Global policies are invisible in resource-filtered listings.** `GET /v1beta/policies/?resource=…` never returns policies with no resource scope. Use diagnostics to see the real candidate set for a request.
4. **`skip` is not `allow`.** It only appears in `and`/`or` batches for actions that were not evaluated.
5. **Unregistered services still evaluate.** A typo in `action.service` gets default metadata (`forbid` priority), not an error, and matches no scoped policy, so it is an implicit deny that is easy to misread as a policy bug. Policy writes are not checked against metadata either, so a policy with a misspelled action or resource type is stored without error and never matches.
6. **Changing `principalIdClaim` changes every principal id.** Existing `Principal::"…"` policies stop matching. Plan the claim (use `oid` on Entra) before seeding.
7. **Groups claim gaps.** Users whose token carries no `groups` (claim disabled, or Entra group overage) fail every `principal in Group::…` policy. See [oidc-and-token-audiences.md](oidc-and-token-audiences.md#principal-id-and-groups-claims).

## Administrators (meta-permissions)

The reserved `permissions` service has five actions. They are enforced only when `openId.enabled: true`.

| Action | Gates |
|---|---|
| `permissions:view` | `GET /v1beta/policies/…`; authorization requests whose resolved `principal` id differs from the caller's (without it, a single request gets `403`, a batch gets per-action `deny`) |
| `permissions:edit` | `PUT /v1beta/policies/`, `PUT /v1beta/policies/batch/`, `DELETE /v1beta/policies/{id}` |
| `permissions:meta:read` | `GET /v1beta/services/…` |
| `permissions:meta:write` | `PUT`/`DELETE /v1beta/services/…` |
| `permissions:diagnostics` | `POST /v1beta/diagnostics/authorize/` and `…/batch/` (checked even for self) |

The [source skill package](../../../NOTICE.md) listed these as `permissions:view`, `permissions:edit`, `permissions:meta`, `permissions:diagnostics`. A policy for `permissions:meta` matches no gate; use `meta:read` and `meta:write` (verified in `crates/permission-service/src/service/authorization.rs` and `deployment.md`).

Seed a small platform-admin group, and a separate auditor group without write rights:

```yaml
database:
  init:
    policies:
      - policy: 'permit(principal in Group::"<platform-admins-group-id>", action in [Action::"permissions:view", Action::"permissions:edit", Action::"permissions:meta:read", Action::"permissions:meta:write", Action::"permissions:diagnostics"], resource);'
        order: 10
        comment: "Platform administrators: full policy and metadata management"
      - policy: 'permit(principal in Group::"<permission-auditors-group-id>", action in [Action::"permissions:view", Action::"permissions:meta:read", Action::"permissions:diagnostics"], resource);'
        order: 10
        comment: "Auditors: read-only"
```

An `action in [...]` list with more than one action leaves the action scope unset, so these policies are fetched for every request and Cedar decides. That is correct but slightly slower; one policy per action keeps them scoped.

Delegate administration of one subtree without letting the delegate touch others:

```text
permit(
  principal in Group::"<projects-admins-group-id>",
  action == Action::"permissions:edit",
  resource in folder::"https://<bucket-host>/Projects/"
);
```

`permissions:edit` is matched against the resource the edited policy governs; a policy with no resource scope presents no resource and is not covered by this grant. Pair it with `permissions:view` on the same subtree so the delegate can list what they manage. The Storage Permission UI needs exactly this pair plus `permissions:meta:read`.

## Storage access

The default metadata seed registers `storage-service` actions `read` and `write` on `object` (parent type `folder`) and `folder`. Storage's operation-to-action map is `config.permissions.actionNames` in the Storage chart (owned by [../production-stack.md](../production-stack.md)).

```yaml
      # deny wins: evaluated before the permits (order 0 < 10)
      - policy: 'forbid(principal, action == Action::"storage-service:write", resource in folder::"https://<bucket-host>/Released/");'
        order: 0
        comment: "Released content is read-only for everyone"
      - policy: 'permit(principal in Group::"<artists-group-id>", action == Action::"storage-service:read", resource in folder::"https://<bucket-host>/Projects/");'
        order: 10
      - policy: 'permit(principal in Group::"<artists-group-id>", action == Action::"storage-service:write", resource in folder::"https://<bucket-host>/Projects/");'
        order: 10
```

Grant per subtree and per group; avoid principal-unconstrained permits on storage actions.

## Service principals

Name each service by the id its token carries in the configured principal claim (on Entra with `principalIdClaim: oid`, the service principal's object id). Give each one only the actions its code calls.

| Caller | Grant | Why |
|---|---|---|
| Permission Service (outbound, when `notifications.publisher.enabled`) | `event-aggregation-service:publish-event` on `EventType` resources it publishes | publishes `omni.permissions.changed` / `omni.permissions.meta.changed` |
| Permission Service (when `notifications.consumer.enabled`) | `event-consumer-service:consume-durable-queues` | cross-replica invalidation stream |
| A service that checks permissions **for** end users (passes `principal`) | `permissions:view`, narrowed with `context.action.service` | the access-control gate requires it |
| Smoke and conformance identities | only the actions their test touches, removed after the run | test principals are long-lived credentials |

```text
// the Permission Service may publish only its own event types
permit(
  principal == Principal::"<permission-service-sp-id>",
  action == Action::"event-aggregation-service:publish-event",
  resource
)
when { resource.id == "omni.permissions.changed" || resource.id == "omni.permissions.meta.changed" };
```

The `resource.id` condition assumes the event-aggregation service sends the event type as the resource id; confirm the request it makes with diagnostics before relying on it ([../notifications.md](../notifications.md)).

```text
// a storage service principal may ask about any user, but only for storage actions
permit(
  principal == Principal::"<storage-service-sp-id>",
  action == Action::"permissions:view",
  resource
)
when { context.action has service && context.action.service == "storage-service" };
```

These grants go on the Permission Service that guards the target service. If the Notification Service is guarded by the same Permission Service, they all live in one policy store.

## User Info access

With `permission.enabled: true` on the User Info Service, every list and get is checked. The actions are `userinfo:list-users`, `get-user`, `list-user-groups`, `get-user-group`, `list-groups`, `get-group`, `list-group-members`, `get-group-member`, on resource types `User` and `Group` (the `Group` here is a directory record being read, not the caller's membership). Register them before you enable the integration, or every request is refused:

```yaml
database:
  init:
    services:
      - name: "userinfo"
        actions: ["list-users", "get-user", "list-user-groups", "get-user-group",
                  "list-groups", "get-group", "list-group-members", "get-group-member"]
        resourceTypes:
          - type: "User"
          - type: "Group"
```

When you set `database.init.services`, you replace the chart's default seed; keep the `storage-service`, `event-consumer-service`, and `event-aggregation-service` entries you still need.

Let a staff group look up people, without exposing group membership lists:

```text
permit(
  principal in Group::"<staff-group-id>",
  action in [Action::"userinfo:list-users", Action::"userinfo:get-user"],
  resource
);
```

List results are filtered per item with `get-user` / `get-group`, so a policy with a `when` on `resource` attributes (for example `resource.email like "*@example.com"`) narrows what a caller sees inside a page. `/users/me` is never checked.

## Your own service

Register the service, its actions, and its resource types (with `parentType` if ids are URLs), set `evaluationPriority: forbid` unless you need additive exceptions, and seed permits per group. The calling side of integration is in [../../develop/add-your-service.md](../../develop/add-your-service.md).

## Test before you trust

```bash
# which policies would be evaluated, in order, and which priority applies
curl -s -X POST https://permissions.<domain>/v1beta/diagnostics/authorize/ \
  -H "Authorization: Bearer ${ADMIN_TOKEN}" -H 'Content-Type: application/json' \
  -d '{"principal":{"sub":"<user-oid>","info":{"groups":["<group-id>"]}},
       "action":{"service":"storage-service","name":"write"},
       "resource":{"type":"object","id":"https://<bucket-host>/Released/a.usd"}}'
```

For each grant, test one request that must be allowed and one that must be denied (another group, a sibling folder such as `/Projects-archive/`, a write where only read was granted). Do not broaden a permit to make a test pass; find the scope that is missing. Use `comment` to record why each policy exists and who approved it. For config-file deployments, keep the policy YAML in reviewed source control.
