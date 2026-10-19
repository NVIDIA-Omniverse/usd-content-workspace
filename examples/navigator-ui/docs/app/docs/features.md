(storage-navigator-app-features-features)=
# Features

Once connected and authenticated, Storage Navigator exposes the storage stack's content through a folder tree, switchable content views, uploads, tags, and file versioning. Every feature on this page operates against whichever storage stack is currently connected; see {ref}`Architecture <storage-navigator-app-architecture-architecture>` for how a stack is discovered and connected.

(storage-navigator-app-features-path-tree-navigation)=
## Path Tree Navigation

The left-hand tree shows the connected stack's top-level storage addresses and lets the user expand folders to browse into subfolders. When {ref}`default discovery <storage-navigator-app-configuration-configuration>` is configured, every storage service the discovery endpoint advertises appears as a top-level node; otherwise, the tree is rooted at the connections the user adds manually.

(storage-navigator-app-features-content-views)=
## Content Views

The main panel lists the contents of the selected folder in one of two switchable layouts: a grid of thumbnails or a flat table with file details. Selecting one or more items in either view surfaces their details, including tags and, when available, thumbnails, in a details panel, and opens context-menu actions for the selection: rename, delete, copy, and paste.

(storage-navigator-app-features-uploads)=
## Uploads

Files and folders can be uploaded by dragging them onto the content view or by picking them from the operating system's file or folder picker. Uploading a folder uploads every file underneath it; intermediate folders are created implicitly as the files inside them are uploaded, rather than as a separate step.

Uploads run in parallel, batched at the {ref}`configured batch size <storage-navigator-app-configuration-configuration>` (25 items by default), with a combined progress indicator showing bytes transferred, total size, and transfer rate across the whole batch. When some items in a batch fail and others succeed, Storage Navigator reports the upload as partially complete and lists which items failed, rather than treating the whole batch as a failure.

(storage-navigator-app-features-tags)=
## Tags

Selected files and folders can be tagged with key/value metadata, which is displayed alongside other file details and can be edited from the details panel.

(storage-navigator-app-features-versioning)=
## Versioning

For storage stacks that support it, Storage Navigator shows the version history of a selected file, letting the user browse prior versions and see when each one was last modified.

(storage-navigator-app-features-clipboard-and-context-actions)=
## Clipboard and Context Actions

Copy and paste, rename, delete, and new-folder creation are available from context menus on the tree and the content views, operating on either the current selection or the folder under the cursor.

(storage-navigator-app-features-see-next)=
## See next

- {ref}`Permissions Integration <storage-navigator-app-permissions-permissions-integration>`. The optional permission-management panel available for a selected path.
