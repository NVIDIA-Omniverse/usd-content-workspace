(userinfo-service-directory-scoping-directory-scoping)=
# Directory Scoping

By default User Info Service serves the whole tenant directory: every user and group the
identity provider exposes to it. Directory scoping narrows that view so the service serves only
the users and groups assigned to a single **enterprise application** in Microsoft Entra ID,
instead of the entire tenant. This page describes what scoping changes about the served data,
how the scope is determined, the directory permissions it needs, and how the replica is rebuilt
when scoping is switched on or off.

Scoping is off by default. When it is off, nothing on this page applies and the service behaves
as described in {ref}`Directory Synchronization and Data Freshness <userinfo-service-directory-synchronization-directory-synchronization-and-data-freshness>`.

(userinfo-service-directory-scoping-why-scope)=
## Why Scope the Directory

Most tenants contain many more users and groups than any single application needs. Restricting
the directory to one enterprise application's assignments keeps the served data aligned with what
that application's own consumers already expect to see. In particular, it matches the view used by
the {ref}`Permission Service <userinfo-service-authorization-authorization-with-the-permission-service>`, which only ever sees the groups an enterprise
application places into a user's access token. A group that is not assigned to the application
therefore never appears in User Info Service either, so the two services agree on which groups
exist for authorization purposes.

(userinfo-service-directory-scoping-what-changes)=
## What Scoping Changes About the Served Directory

Scoping restricts **groups and group memberships**; it does not restrict users.

- **Groups.** Only the groups assigned to the chosen enterprise application are served. A group
  that exists in the tenant but is not assigned to the application is never returned by any group
  request, and never appears in a user's group memberships.
- **Memberships.** Only the direct members of the assigned groups are served. Membership queries
  reflect the assigned groups only.
- **Users.** Users continue to be synchronized from the whole tenant exactly as they are without
  scoping. Scoping does not hide users; it only limits which groups and memberships are visible.
  The one addition to the user set is described in
  {ref}`Assigned Service Principals as Users <userinfo-service-directory-scoping-service-principals>`
  below.

(userinfo-service-directory-scoping-how-scope-determined)=
## How the Scope Is Determined

The scope is read from the enterprise application itself rather than typed in by an operator.
Each synchronization cycle, the writer reads the list of principals assigned to the application
and sorts them by kind:

- **Assigned groups** define the set of groups the service will serve. The writer then reads each
  assigned group and enumerates its direct members, and serves exactly that set.
- **Assigned service principals** are surfaced as users (see below).
- **Directly assigned users** are ignored here, because they are already served through the normal
  whole-tenant user synchronization.

Because the allowed groups are discovered from the application's assignments — not configured by
name — a misspelled or non-existent group cannot creep into the configuration. Whatever is
assigned to the application is what gets served.

Unlike the whole-tenant model, scoped synchronization does not track incremental changes to
groups. Each cycle re-reads the current set of assigned groups and their members and replaces the
stored set with it, so groups that are added to, removed from, or renamed within the application's
assignments reconcile automatically on the next cycle. Users are unaffected and keep synchronizing
through the usual incremental change-tracking path. For the underlying synchronization model, see
{ref}`Directory Synchronization and Data Freshness <userinfo-service-directory-synchronization-directory-synchronization-and-data-freshness>`.

(userinfo-service-directory-scoping-service-principals)=
## Assigned Service Principals as Users

Enterprise applications can be assigned to other applications' service principals — the directory
identities that represent applications and automated agents — not only to people. When scoping is
enabled, each assigned service principal is surfaced as an ordinary directory user so that callers
can look it up alongside real people. A service principal has no email address or username, so its
directory identifier is used as its username, and it carries only an identifier and a display name.

These surfaced users behave like any other user in list and get responses, but:

- They are never added to any group's membership.
- If a service principal happens to share an identifier with a real synchronized user, the real
  user is kept and is not overwritten.

Surfacing service principals as users is on by default and can be turned off, in which case only
the assigned groups and their members are affected by scoping and no service principals are added.

(userinfo-service-directory-scoping-permissions)=
## Directory Permissions and the Startup Safety Check

Reading an application's assignments requires the service to identify which enterprise application
to inspect. By default it inspects **its own** application — the identity it already authenticates
as — which needs no additional directory permission, because an application is always allowed to
read its own assignments. This is the recommended configuration.

The scope can instead be pointed at a **different** application by supplying that application's
service principal identifier. Reading another application's assignments requires the
`Application.Read.All` directory permission; without it the identity provider refuses the request.
For the full set of directory permissions the service uses, see
{ref}`Microsoft Entra ID Integration <userinfo-service-identity-provider-microsoft-entra-id-integration>`.

To avoid silently serving the wrong directory, the writer performs a safety check at startup: it
attempts to read the first page of the configured application's assignments before it begins
serving. If that read is refused — for example because the scope points at a different application
and the extra permission has not been granted — the writer stops with an error instead of falling
back to the whole tenant. Scoping therefore either serves the intended narrowed directory or does
not start at all.

(userinfo-service-directory-scoping-configuration)=
## Configuration and Prerequisites

Scoping is controlled by the settings below.

| Setting | Environment variable | Helm value (`appRoleAssignments.*`) | Default | Description |
|---|---|---|---|---|
| Enable scoping | `APP_ROLE_ASSIGNMENTS_ENABLED` | `enabled` | `false` | Restrict the served directory to one enterprise application's assignments. |
| Application to inspect | `APP_ROLE_ASSIGNMENTS_SERVICE_PRINCIPAL_OBJECT_ID` | `servicePrincipalObjectId` | _(unset)_ | Identifier of the enterprise application's service principal to read. Leave unset to inspect the service's own application, which needs no extra permission. Setting it to a different application requires `Application.Read.All`. |
| Surface service principals | `APP_ROLE_ASSIGNMENTS_INCLUDE_SERVICE_PRINCIPALS` | `includeServicePrincipals` | `true` | Surface the application's assigned service principals as users. |

Scoping is built entirely from the synchronized replica; it has no live-passthrough form.
Enabling it therefore requires the replica to be enabled for the directory as a whole, for groups,
and for memberships, so that group and membership requests are answered from the scoped replica
rather than falling through to the identity provider and bypassing the scope. These prerequisites
are checked when the service starts and when the Helm chart is rendered; a contradictory
configuration is rejected with a clear message rather than being quietly overridden, so a
deliberately disabled membership replica keeps its meaning. For the replica settings themselves,
see {ref}`Directory Synchronization and Data Freshness <userinfo-service-directory-synchronization-directory-synchronization-and-data-freshness>`.

(userinfo-service-directory-scoping-helm)=
### Configuring Scoping in the Helm Chart

In the synchronized directory deployment, add the `appRoleAssignments` section to
`userinfo-values.yaml` and enable the required replica settings alongside it. The recommended form
inspects the service's own application and needs no extra directory permission:

```yaml
providers:
  cache:
    enabled: true
    groups:
      enabled: true
    memberships:
      enabled: true

appRoleAssignments:
  enabled: true
  includeServicePrincipals: true
```

Because scoping requires the group and membership replica, `providers.cache.memberships.enabled`
must be set to `true` even though it defaults to `false` in the standard deployment; the chart
refuses to render if scoping is enabled without it. To inspect a **different** enterprise
application instead of the service's own, set its service principal identifier and grant the
`Application.Read.All` permission to the service's application:

```yaml
appRoleAssignments:
  enabled: true
  servicePrincipalObjectId: "{SERVICE_PRINCIPAL_OBJECT_ID}"
```

Leave `servicePrincipalObjectId` unset to inspect the service's own application. Install or upgrade
the release with the same commands used for the synchronized directory deployment in
{ref}`User Info Service Configuration <userinfo-service-deployment-configuration>`.

(userinfo-service-directory-scoping-switching)=
## Switching Scoping On or Off

Turning scoping on or off changes which data belongs in the replica, so the service records
whether scoping was in effect when the stored replica was built. When the service starts and finds
that this recorded state differs from the current configuration, it discards the stored replica
and performs a fresh full synchronization:

- **Switching on** rebuilds the replica with only the assigned groups and their members.
- **Switching off** rebuilds the replica with the whole tenant's groups and memberships.

While scoping stays on, changes to the application's assigned set need no special handling — they
are picked up automatically on the next synchronization cycle, because each cycle re-reads the
current assignments. Readers continue to serve the last complete directory they hold until the new
replica is available, as described in
{ref}`Directory Synchronization and Data Freshness <userinfo-service-directory-synchronization-directory-synchronization-and-data-freshness>`.

To overlay additional users, groups, and memberships that do not come from the identity provider
at all — independently of whether scoping is enabled — see
{ref}`Static Directory Overlay <userinfo-service-static-directory-static-directory-overlay>`.

(userinfo-service-directory-scoping-references)=
## References

- [Application and service principal objects in Microsoft Entra ID][app-sp] — enterprise
  applications, service principals, and how assignments relate to them.
- [Assign users and groups to an application][assign] — how principals are assigned to an
  enterprise application.
- [List appRoleAssignedTo][app-role-assigned-to] — the assignments read to determine the scope.
- [Get servicePrincipal][get-sp] — reading an application's own service principal without an
  additional permission.
- [Microsoft Graph permissions reference][permissions] — the `Application.Read.All` permission
  required to inspect a different application.

[app-sp]: https://learn.microsoft.com/en-us/entra/identity-platform/app-objects-and-service-principals
[assign]: https://learn.microsoft.com/en-us/entra/identity/enterprise-apps/assign-user-or-group-access-portal
[app-role-assigned-to]: https://learn.microsoft.com/en-us/graph/api/serviceprincipal-list-approleassignedto
[get-sp]: https://learn.microsoft.com/en-us/graph/api/serviceprincipal-get
[permissions]: https://learn.microsoft.com/en-us/graph/permissions-reference

(userinfo-service-directory-scoping-see-next)=
## See next

- {ref}`Static Directory Overlay <userinfo-service-static-directory-static-directory-overlay>`
