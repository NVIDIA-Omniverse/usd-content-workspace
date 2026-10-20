# USD workflows on the stack (end users)

> **Applies to:** Kit applications, the Client Library, Storage Navigator, and the Live Edit client · **Released:** Developer Guide 1.3.0 inventory; docs pin Client Library 2.72, the [source skill package](../../NOTICE.md)'s client examples target 2.74 · **This repo's snapshot:** Navigator UI 1.0.4 (`package.json`; `CHANGELOG.md` also has an unreleased `# 1.0.5` section) · **Sources:** the source skill package, upstream omni-client-library skill; [`examples/`](../../../../examples/)

This page is for people who open, edit, and share USD scenes, and for those who support them. The services are invisible when they work: Kit sees folders and files, opens layers, and saves.

## Connect a Kit application

1. Get the Discovery URL from your administrator, for example `https://discovery.example.com`.
2. Set it before Kit starts. In the Kit launch script (for example `omni.app.full.sh`), before the `${EXEC:-exec}` line:

   ```bash
   export OMNI_STORAGE_DISCOVERY=https://<discovery-host>
   ```

   Use `http://` only for a local, port-forwarded test deployment.
3. Launch Kit, choose **Add New Connection** in the Content Browser, and enter the storage address your administrator gave you (for example an S3 or Azure address exposed by the Storage Service).
4. Sign in when prompted. Kit obtains a token from your organization's identity provider; the stack never asks for your password directly.
5. Browse, open, edit, and save. On Kit 109.0.1 a known issue prevents adding the connection in the Content Browser; use **File > Open** with the address instead.

The Client Library inside Kit sends file operations for recognized addresses to the Storage Service it finds through Discovery. Its version decides which features work; check `omni.client.get_version()` in the running application. See [../components/client-library.md](../components/client-library.md).

## What users experience, and which service provides it

| User action | Behind it | If it fails, check |
| --- | --- | --- |
| See the server's top-level folders | Storage `ListTopLevelAddresses`; endpoints with `listInTopLevelAddresses` | Discovery URL, endpoint configuration |
| Open, save, copy, move files | Storage file and folder APIs | Token, permissions, backend credentials; cross-backend copy and move return 501 by design |
| See earlier versions of a file | Storage versioning API, where the backend has versioning enabled | Backend and endpoint versioning settings |
| See other users' changes appear | Notifications: Storage publishes, Event Consumer delivers | Notifications deployed and Storage event publishing enabled |
| Edit the same layer together live | Live Edit Service and ovlive client runtime | Live Edit registered in Discovery; client network path to its gRPC port |
| See previews in browsers | Thumbnail Service | Thumbnail workers and their access to Storage |
| Get "access denied" | Permission Service policies | Policy diagnostics; group membership through User Info |
| See a user's name instead of an ID | User Info Service | Directory sync and Entra app permissions |

Runbooks for each failure: [../operate/runbooks.md](../operate/runbooks.md).

## USD content habits that work well

- **Keep references relative inside a project folder.** Relative asset paths survive moving the project between endpoints, buckets, or packages; absolute addresses tie a scene to one deployment.
- **Use one address form consistently.** The snapshot Storage Service normalizes addresses by default, and released 1.2.3 does not. Two spellings of an address can resolve to different objects on one and the same object on the other; do not rely on either.
- **Large layers and textures:** when saving large files fails while small files work, suspect ingress body-size or timeout limits and the Storage Service write and redirect size thresholds before suspecting USD.
- **Checkpoints and versions** depend on backend versioning. Ask your administrator whether the endpoint keeps versions before relying on them for recovery.
- **Live sessions** follow the client application's live-layer workflow; see [../operate/live-edit.md](../operate/live-edit.md) for what the service keeps and for how long.

## Browse without Kit

- **Storage Navigator UI** — a browser file manager: browse, upload, download, rename, duplicate, delete, and edit tags. Version 1.0.4 uses Discovery v2 and falls back to v1. The snapshot source also downloads a selection as one ZIP when the deployment advertises a same-site archive service, and prefers the v1beta metadata API; whether the released 1.0.4 image does is not recorded. See [../deploy/navigator.md](../deploy/navigator.md).

## Packaging and caching

Versioned asset packages (ovpackage) and caches (OVCC, OVDC, Hub Workstation Cache) sit beside this stack. See [companion-products.md](companion-products.md).
