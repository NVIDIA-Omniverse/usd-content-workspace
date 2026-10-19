(storage-navigator-app-permissions-permissions-integration)=
# Permissions Integration

Storage Navigator can show a permission-management panel for the selected path, letting an authenticated user view and edit access control for that file or folder. This panel is **available only when the connected storage stack advertises a permission service through discovery**; it is not a setting an operator turns on or off directly. When no permission service is advertised, or the user is not authenticated, the panel does not offer permission management.

(storage-navigator-app-permissions-when-the-panel-is-shown)=
## When the Panel Is Shown

For the currently selected path, Storage Navigator looks up two endpoints from the connected stack's discovery document: a permission service and a userinfo endpoint. What the user sees depends on what discovery returns and whether the user is authenticated:

- **No permission service advertised.** The panel shows a short notice that permission management is not supported for this connection, and no further permission calls are made.
- **A permission service is advertised, and the user is authenticated.** The panel renders permission management for the selected path, scoped to that path as either a file or a folder resource, using the user's current access and id tokens.
- **The user is not authenticated.** No permission panel is shown, since permission management always acts on behalf of the signed-in user.

(storage-navigator-app-permissions-optional-integration)=
## Optional Integration

The permission-management UI itself is delivered as a separate, optional package. When that package is not installed, Storage Navigator falls back to the same "not supported" notice described above for a connection with no permission service advertised, so the application still builds and runs without the optional package present. This lets a deployment with no permission service in its storage stack, or one that intentionally omits the optional package, run Storage Navigator without permission management, while a deployment with both the package and an advertised permission service gets full permission management with no separate configuration switch.

(storage-navigator-app-permissions-see-next)=
## See next

- {ref}`Troubleshooting <storage-navigator-app-troubleshooting-troubleshooting>`. Diagnosing configuration and connection problems.
