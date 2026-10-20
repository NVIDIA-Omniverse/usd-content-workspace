(userinfo-service-static-directory-static-directory-overlay)=
# Static Directory Overlay

By default User Info Service serves only what it synchronizes from the identity provider. The
static directory overlay lets operators serve *additional* users, groups, and memberships that
do not come from the identity provider at all — for example service accounts, automation
identities, or locally managed groups that have no counterpart in Microsoft Entra ID. These
extra entities are served alongside the synchronized directory through the same REST and gRPC
endpoints, so callers cannot tell whether a record came from the identity provider or from the
overlay.

The overlay is off by default. When no overlay is supplied, nothing on this page applies and the
service behaves exactly as described in
{ref}`Directory Synchronization and Data Freshness <userinfo-service-directory-synchronization-directory-synchronization-and-data-freshness>`. The overlay is
independent of {ref}`Directory Scoping <userinfo-service-directory-scoping-directory-scoping>` — it can be used with or without
scoping, and its entries are added on top of whatever the scoped or whole-tenant directory
serves.

(userinfo-service-static-directory-what-you-can-add)=
## What You Can Add

The overlay accepts three kinds of entries, each with the same shape as the records the service
already serves:

- **Users** — require an identifier, a display name, and a username. An email address, given and
  family name, active status, and job title are optional.
- **Groups** — require an identifier and a display name. A description is optional.
- **Memberships** — attach one or more users to a group. Each membership names a group and lists
  the users that belong directly to it.

Identifiers are chosen by the operator and must be unique within each kind. They are what
callers use to look an entity up, and what memberships reference. If the same identifier appears
twice in the same list, the last occurrence wins.

(userinfo-service-static-directory-when-read)=
## When the Overlay Is Read

The overlay is read once, when a reader starts, and then held for the life of that instance.
Changing the overlay therefore requires the readers to be restarted before the change takes
effect; the {ref}`Helm configuration <userinfo-service-static-directory-helm>` below does this
automatically on upgrade.

Only readers — the instances that serve the REST and gRPC APIs — use the overlay. The writer,
which synchronizes the identity provider into the shared store, never serves requests and so
ignores the overlay entirely. Overlay entries are **never** written to the shared database. As a
result a full resynchronization by the writer, which rebuilds the synchronized data from
scratch, cannot erase the overlay, and adding an overlay never changes what is stored for other
readers.

The overlay is also independent of caching. Its entries are returned on every read whether or
not the service caches the directory locally, and regardless of which kinds of data are cached.
This means the overlay works in both the synchronized-directory deployment and the live
pass-through mode described in {ref}`User Info Service Configuration <userinfo-service-deployment-configuration>`.

(userinfo-service-static-directory-how-combined)=
## How Overlay Entries Combine With Synchronized Data

Overlay entries are merged with the synchronized directory on every read. The rules below
describe exactly what a caller sees.

(userinfo-service-static-directory-precedence)=
### Precedence

When an overlay entry shares an identifier with a synchronized user or group, the overlay entry
wins:

- A request for that single user or group returns the overlay record.
- In a listing, the overlay record appears and the synchronized record with the same identifier
  is dropped, so the entity is never listed twice.

This lets an operator deliberately override a synchronized record — for example to correct a
display name — as well as add entirely new entities.

(userinfo-service-static-directory-existence)=
### Existence

An entity that exists only in the overlay is treated as a real directory entry:

- Looking it up by identifier succeeds.
- Listing its group memberships (for a user) or its members (for a group) returns the overlay
  set, even when the identity provider has never heard of that entity.

(userinfo-service-static-directory-memberships)=
### Memberships

The members of a group — and the groups a user belongs to — are the combination of the overlay
memberships and whatever the synchronized directory reports, with duplicates removed. A single
overlay membership may freely mix identity-provider and overlay identifiers: it can add a
synchronized user to an overlay group, an overlay user to a synchronized group, or any other
combination. Identifiers a membership references but does not itself define are resolved against
the synchronized directory.

A membership that references an entity which exists in neither the overlay nor the synchronized
directory is ignored for that entity and recorded in the service logs at startup. It does not
prevent the service from starting, and the rest of the membership still applies.

(userinfo-service-static-directory-listing-and-filtering)=
### Listing, Filtering, and Pagination

Overlay entries obey the same filtering and pagination rules as synchronized data:

- Name and email filters (`display_name` prefix and substring matching, exact `email` matching)
  apply to overlay entries too, so a filtered listing includes only the overlay entries that
  match.
- In listings, overlay entries are returned first, ordered by identifier, followed by the
  synchronized entries. Pagination works across the combined result using the usual opaque
  cursor, so callers page through overlay and synchronized entries seamlessly without handling
  them differently.
- The requested page size is treated as an upper bound. A page may contain fewer entries than
  requested when a synchronized entry is dropped because an overlay entry overrides it.

(userinfo-service-static-directory-current-user)=
## The Current-User Request

The current-user request (`GET /users/me` and the equivalent gRPC call) identifies the caller
from their access token and returns the matching directory record. When an overlay is in effect,
the caller's identity is matched against the overlay first: a service account defined only in the
overlay can therefore call the current-user endpoint and receive its own overlay record. Only
when no overlay user matches the caller's identity does the service fall back to the
synchronized directory.

(userinfo-service-static-directory-configuration)=
## Configuration

The overlay is enabled and populated entirely through the Helm chart. The chart takes the users,
groups, and memberships authored as structured values, renders them into a configuration file,
and mounts that file read-only into each reader; the reader loads it at startup. There is no
separate environment variable for operators to set by hand.

| Setting | Helm value (`staticDirectory.*`) | Default | Description |
|---|---|---|---|
| Enable the overlay | `enabled` | `false` | Render and mount the overlay, and load it into the readers. |
| Users | `users` | `[]` | Additional users to serve. Each requires `id`, `display_name`, and `username`; `given_name`, `family_name`, `email`, `active`, and `job_title` are optional. |
| Groups | `groups` | `[]` | Additional groups to serve. Each requires `id` and `display_name`; `description` is optional. |
| Memberships | `memberships` | `[]` | Direct group memberships. Each entry has a `group_id` and a list of `user_ids`; either may reference overlay or synchronized identifiers. |

(userinfo-service-static-directory-helm)=
### Configuring the Overlay in the Helm Chart

Add the `staticDirectory` section to `userinfo-values.yaml`, set `enabled` to `true`, and list
the entities to serve. The example below adds two service accounts, a locally managed group, and
the memberships that place the service accounts into it:

```yaml
staticDirectory:
  enabled: true
  users:
    - id: "svc-ci-bot"
      display_name: "CI Bot"
      username: "svc-ci-bot"
      email: "ci-bot@example.com"
    - id: "svc-release-bot"
      display_name: "Release Bot"
      username: "svc-release-bot"
  groups:
    - id: "local-admins"
      display_name: "Local Admins"
      description: "Statically managed"
  memberships:
    - group_id: "local-admins"
      user_ids:
        - "svc-ci-bot"
        - "svc-release-bot"
```

Install or upgrade the release with the same commands used for your deployment in
{ref}`User Info Service Configuration <userinfo-service-deployment-configuration>`. Because the readers read the overlay only at
startup, the chart is wired so that changing any `staticDirectory` value on an upgrade rolls the
reader pods automatically, ensuring the new overlay takes effect. Leaving `staticDirectory.enabled`
as `false`, or leaving all three lists empty, serves the synchronized directory unchanged.

The `group_id` and `user_ids` in a membership may point at either overlay entities defined above
or entities synchronized from the identity provider, in any combination. To attach an overlay
service account to a group that already exists in the identity provider, use that group's
identity-provider identifier as the `group_id` and the overlay user's `id` in `user_ids`.

(userinfo-service-static-directory-see-next)=
## See next

- {ref}`Scaling and High Availability <userinfo-service-scaling-scaling-and-high-availability>`
