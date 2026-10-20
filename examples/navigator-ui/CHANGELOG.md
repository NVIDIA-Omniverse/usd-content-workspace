# Storage Navigator Changelog

# 1.0.5

- Build the container image on the public nginxinc/nginx-unprivileged image by default; NVIDIA builds still use simple-nginx through the SERVE_IMAGE build argument.
- Fix "URI malformed" error when listing a folder containing a file whose name includes a percent sign.
- Fix renaming a folder in the Path Tree so it stays in its parent folder, and refresh expanded Path Tree nodes.
- Build the container image and pin local development (`.nvmrc`) on Node.js 22, which the app and its tests require.

# 1.0.4

- Relicense Storage Navigator under Apache-2.0.
- Update @omniverse/storage-permission-ui to 0.0.24.
- Add discovery v2 API resolution with v1 fallback.
- Append storage resource names to discovery v2 API request paths.
- Fall back to discovery v1 on any discovery v2 probe failure and log the reason.
- Fix Path Tree rendering and selection glitches when the same resource address appears in more than one location in the tree.
- Fix duplicate files when notification and listStat resource addresses encode parentheses differently.
- Decode resource addresses used by the thumbnail query cache.
- Add optional overwrite control for file copy and move operations. OMPE-106333
- Retry transient failures during direct, redirected, and multipart file uploads.
- Prefer the top-level v1beta metadata API and fall back to v1alpha when a deployment does not advertise it. OMPE-105917
- Download selected files and folders as a single ZIP archive through the archive service when the deployment advertises it on the same site as the app.
- Update react-router-dom to 7.18.4.
- Update simple-nginx to 1.1.7.
- Add service documentation covering architecture, authentication, configuration, deployment, and troubleshooting.

# 1.0.3

- Fix issue where empty folders are removed on rename/move.
- Make the HTTP request timeout configurable via `config.json` (`requestTimeout`, defaults to 30000 ms).
- Make the batch size configurable via `config.json` (`batchSize`, defaults to 25).
- Fix duplicate file entries when SSE notification resource addresses are percent-encoded differently from cached addresses.
- Clear table/grid item selection when users click empty content space.
- Load all file version pages before sorting by modified date unless the service returns newest-first order.
- Prevent false error notification when downloading zero byte files.
- Update React to 19.2.7 and Mantine to 9.4.1
- Update simple-nginx to 1.1.6.
- Display file thumbnail for omni.thumbnail.associated notifications. OMPE-64164
- Display file dates in local time.
- Invalidate the version query cache on file upload, copy or move operations.
- Fix chevron click to reliably toggle folder expansion in Path Tree. OMPE-94351
- Cache current folder metadata in context menu via query client. OMPE-98566
- Display file thumbnail if available in path grid view.
- Update clipboard confirmation dialog title and button. OMPE-94343
- Call complete after a successful redirected upload. Relax response code checks to accept any 2xx response as success.
- Call delete folder after a successful folder move operation.
- Show current user email next to connection in path tree. NVBUG-6371364
- Remember last selected side panel tab across path selections. NVBUG-6371365
- Replace the allotment package with the Mantine Splitter component for resizable panels.

# 1.0.2

- Fix duplicate entry on omni.storage.created event OMPE-85347
- Fix for file/folder decoding at render OMPE-85348
- Update copy to use the production API route v1alpha instead of v1testcopy
- Fixed path encoding behavior in clipboard by URL-encoding copied/pasted file and folder names consistently when constructing destination paths
- Improve error message for notification service errors
- Update simple-nginx to 1.1.0

# 1.0.1

- Update simple-nginx to 1.0.2.
- Update @omniverse/storage-permission-ui to 0.0.13
- Reduce the minimum width of the content pane to 200px
- Display the context menu for folders and top-level addresses in the tree
- Improve folder navigation and handle deleted folders gracefully
- Add About dialog to display the application version and OSS licenses. OMPE-87939
- Fix the folder thumbnail size to be aligned with file thumbnail on the grid view
- Throttle concurrent requests during folder copy/paste and delete operations
- Fix tree selection highlight width and auto-scroll to selected node
- Update Connections tree node label to Omniverse Storage

# 0.0.49 - 2026-03-11

- Add option to multi-select table/grid items using shift-select and ctrl-select. OMPE-84457
- Improve selection performance in the grid and table by replacing repeated linear lookups with O(1) map lookups OMPE-84453
- Add batching for context menu metadata API calls, add 'checking permissions' message for context menu while metadata is loading OMPE-84631

# 0.0.48 - 2026-02-27

- Update simple-nginx to 0.2.7

# 0.0.47 - 2026-02-27

- Update @omniverse/storage-permission-ui to 0.0.12
- Integrate SRD-compliant SBOM generation

# 0.0.46 - 2026-02-12

- Add default timeout to HTTPProxy objects in the Helm chart

# 0.0.45 - 2026-02-09

- Fixed an error when Navigator tries to refresh the session and throws an error if auth config is not available

# 0.0.44 - 2026-02-09

- Fix regression where resource was being url decoded twice for incoming resources

# 0.0.43 - 2026-02-09

- Encode/decode resource path segments for URLs and display
- add readiness/liveness probes OMPE-79489
- Fixed an error where endpoints were not updated correctly in usePathNavigation.ts

# 0.0.42 - 2026-02-06

- Update error handling in useStorageSubscription
- Add httpProxy.annotations
- Use the default client from Discovery auth-config
- Fixed incorrect call signature for `Batch.onItemError` in useStorageSubscription.tsx
- Remove duplicate jobs in .gitlab-ci.yml

# 0.0.41 - 2026-02-04

- Update package.json version

# 0.0.40 - 2026-02-04

- Update default image repository to nvcr.io/nvidia/omniverse

# 0.0.39 - 2026-02-03

- Update simple-nginx to 0.2.6

# 0.0.38 - 2026-02-02

- Update react-router-dom to 7.13.0

# 0.0.37 - 2026-01-17

- Add Ingressii to Navigator

# 0.0.36 - 2026-01-14

- Update storage-permission-ui library to 0.0.11

# 0.0.35 - 2026-01-13

- Do not select the connection when user triggers its context menu
- Prevent metadata call for a connection node
- Ensure connection url has a trailing slash
- Support connecting to deployments without authentication
- Added .Values.image.pullSecrets section
- Fix volume mount for config map; add Update Strategy to deployment

# 0.0.34 - 2025-12-12

- Add scrollbar for Permissions and Tags tabs
- Update storage-permission-ui library to 0.0.9
- Update fileFolder service and useDeleteMutation hook to use storage v23 delete folder
- Disable paste when dest folder = source folder, display error on paste to child of source folder
- Use canonical names for Kubernetes services

# 0.0.33 - 2025-11-27

- Use URLSearchParams in favor of string concatenation for query params

# 0.0.32 - 2025-11-18

- Update Navigator to use storage api move endpoint
- Exclude src/test-utils from SonarQube scanning
- Add SONAR_EXCLUSIONS to exclude unit tests from scanning
- Update Navigator to use storage api create folder method

# 0.0.31 - 2025-11-14

- Update react-router-dom version to 7.9.5

# 0.0.30 - 2025-11-14

- Update NGC org for nspect:register:helm
- Register containers and Helm charts in nSpect for the initial release
- Update volume mount, remove tag auto generate
- Add NPM mirroring
- Update nginx config in configmap, bump version
- Add ATTRIBUTIONS.md
- Update react-router-dom, update base image, bump version
- Update Dockerfile to use approved base image

# 0.0.26 - 2025-10-06

- Update Notification endpoint version to use v1beta

# 0.0.25 - 2025-10-06

- Bug fixes from s3 testing
- Update @omniverse/storage-permission-ui to 0.0.8

# 0.0.24 - 2025-09-19

- Update Storage API routes to use v1alpha
- Add NSPECT security scanning support OMPE-64082
- Limit retries to notifications for network errors
- Add storage-api-dev publishing for NGC dev environment OMPE-63670
- Include additionalHeaders when download via a redirect
- Migrate storage-navigator to CDS Components with Dual Publishing OMPE-62750

# 0.0.23 - 2025-09-13

- Fix helm:lint image reference to use CI_REGISTRY
- Modernize CI pipeline with CDS v0.16.8 and CI Components
- chore(release): Bump version to 0.0.22 for release test
- fix(ci): Correct undefined need for helm:package
- refactor(ci): Standardize job naming convention
- chore(ci): Remove latest tag and force push from staging publish
- storage-navigator CI: align envsubst install with UCC pattern OMPE-62750
- storage-navigator CI: remove unsupported build-pre-build-script OMPE-62750
- storage-navigator CI: fix docker-push inputs and pre-build script OMPE-62750
- Fix "Unable to locate package gettext" error in build job
- Migrate storage-navigator to CDS Components and align versions

# 0.0.20 - 2025-08-28

- Update afterResponse handler to refresh token on 403 response with retry
- Update dropzone overlay position to fixed
- Disable SonarQube scans temporarily
- Add security scans

# 0.0.19 - 2025-08-20

- Add default value for acl

# 0.0.18 - 2025-08-20

- Display thumbnail in path content grid view

# 0.0.17 - 2025-08-18

- Fix for Table/Grid virtualization
- Add item count to path content bar
- Implement Multipart Upload
- fix minor UI alignment issue for tag action buttons
- Add completed file count to folder download progress, remove download preference
- Throw error if downloaded file size does not match server size, update remaining time string
- Add transfer rate to file action notifications
- Implement retry limit on 401  responses
- set focus to tag key when entering tags

# 0.0.16 - 2025-07-23

- Proactive token refresh
- Add Tagging Integration

# 0.0.15 - 2025-07-15

- Update Storage Permission UI to 0.0.7
- Clear global selected paths after a successful delete operation
- Fixed an issue where multiple batch flush operations could be run concurrently
- Fixed an issue where tabs content in the details panel couldn't be scrolled vertically
- Fixed an issue where error notifications popped up for tree elements on every render
- Persist file action notifications if browser tab does not have focus
- Fixed an issue where trailing slashes were not included for navigation links...

# 0.0.14 - 2025-07-10

- Integrate Storage Permission UI
- Sync table row selection with connection selectedPaths state
- Get permissions for selected paths to disable context menu options
- Display Resolved Access field in the right panel with user permissions
- Rename hooks to use "path" in the name instead of "resource"

# 0.0.13 - 2025-07-08

- Invalidate query cache for source path for cut/paste
- Fixed an issue where the dropzone couldn't be scrolled vertically
- Add file/folder upload via dropzone
- Enable Eslint hooks warnings, update dependencies

# 0.0.12 - 2025-06-27

- Support pasting discovery URL into the connect modal
- fix issue with cut/rename writing to incorrect path, invalidate source path on cut/paste operation
- Restore previous file version using fileObject/copy

# 0.0.11 - 2025-06-11

- Fix for duplicate entries in path content
- pass options to discovery service
- Added notification service integration
- Update DOCKER_REPO in .gitlab-ci.yml after transfer
- remove normalizeUri calls
- Implement multi-selection for the grid view
- Implement a toggle icon for the details panel
- Implement the grid view
- Fixed tree state initialization for top level addresses that don't have a path
- Fixed an issue where multiple requests were made to the API for paginated results
- add batching to file uploads
- Change styling for breadcrumbs in the header
- Display breadcrumbs with the current folder in the header
- Integrate the History Web API
- Implement virtualized tree
- Retrieve OpenID client information from Discovery API
- Refactor dialogs to use @mantine/modals manager
- Update query keys to simplify folder content and version list invalidation
- Integrate versioning API from the Storage Service
- set download preference to body

# 0.0.10 - 2025-05-14

- Implement rename functionality for files and folders
- Implement cut/paste functionality for files and folders
- Add the right panel with stat info for selected files and folders
- Add bucket icon for top level address folders
- Add eslint recommended rules, fix lint warnings/errors
- Add error handling in useFileDownload, fix return types for onClick for upload and download
- Refactor to address warnings from eslint-plugin-react-refresh

# 0.0.9 - 2025-05-08

- Allow to provide scopes not listed in OpenID Connect Configuration

# 0.0.8 - 2025-05-07

- update file operations to use batch processing OMPE-46843
- Update the URL for listing top level addresses
- Add folder download functionality
- Delete files in parallel
- Implement copy-paste functionality
- disable context menu when viewing root folder content OMPE-46065
- file and folder upload OMPE-29082
- remove queue from useFileDownload
- Switch to swc compiler
- Add /api/v1/auth-config endpoint for retrieving the auth configuration

# 0.0.7 - 2025-04-23

- Fixed an issue where the failure state wasn't used for download notifications
- Fixed an issue with URLSearchParams when running tests on newer versions of NodeJS
- Fixed an issue where tree wasn't expanded when the user navigates within the content view
- Fixed issue with missing handler for Add Connection tree node
- Fixed the issue where the tree component didn't work with multiple connections
- Update the name and type of lastModifiedTimestamp field in MetadataSchema
- fix no active connection error
- Simplify useFileActionNotification signatures and add the cancellation state

# 0.0.6 - 2025-04-17

- Select multiple files to download using multiselect, download files as a task

# 0.0.5 - 2025-04-16

- Implement file and folder deletion
- Implement "New Folder" context menu option
- Added file download queue

# 0.0.4 - 2025-04-04

- Virtualize PathContent table
- Implement a context menu for connections in the tree

# 0.0.2 - 2025-03-20

- Disable refetchOnWindowFocus for useFolderContentQuery
- Added PathContent component + other updates
- Add Open Sans fonts to the public folder
- Disable retries and refetches for OpenID page

# 0.0.3 - 2025-03-19

- Fix an issue where endpoints were duplicated in the tree

# 0.0.1 - 2025-03-18

- Publish Docker images and Helm charts to sevan-cesspool
- Load all pages of data for storage service list and listStat
- Use camelCase for fields received from the API
- Fix tree folder expanded state
- Remove mock from capabilities service
- use accessToken in auth header
- Add discovery
- Add CI for testing, building and publishing Docker containers and Helm charts
- add allotment to package.json
- Misc UI related updates
- Implement refresh token functionality
- Integrate OpenID Connect
- add discovery service and useDiscovery hook
- Add global runtime application configuration
- add support for baseUri for folder paths
- Storage Connect, Tree View, Persist to LocalStorage
- additional vitest config, add connectionStore
- Update vitest config to exclude storybook files
- Add vitest code coverage
- Add component templates, update deps
- UI prototype using Mantine/Zustand
