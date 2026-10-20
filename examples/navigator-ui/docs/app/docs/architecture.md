(storage-navigator-app-architecture-architecture)=
# Architecture

Describes the client stack Storage Navigator is built on, what it integrates with, and the request flow from page load to an authenticated API call. This section is always relevant. A client application has no "off" state, so there is no default-behavior toggle to link back to.

(storage-navigator-app-architecture-client-stack)=
## Client Stack

| Area | Overview |
|---|---|
| Deployment unit | A single nginx container serving a pre-built static single-page application bundle |
| Client type | A browser single-page application with no server-side rendering or application logic |
| Data fetching and caching | REST calls to the discovered APIs, cached in the browser for the lifetime of the page to avoid redundant requests |
| Client-side state | Connections, view preferences, and session tokens, persisted to the browser's local storage across sessions |
| Authentication | OpenID Connect authorization code flow with PKCE, performed entirely in the browser |

Storage Navigator has no database, message bus, or background worker of its own. All durable state beyond the current page is either kept in the browser (connections, tokens, view preferences) or lives in the storage stack it talks to.

(storage-navigator-app-architecture-what-storage-navigator-interacts-with)=
## What Storage Navigator Interacts With

| External System | Role |
|---|---|
| **Discovery service** | Resolves a storage stack's REST endpoints, meaning its storage, permission, and thumbnail services, from a hostname or discovery URL the operator or user supplies. |
| **OpenID Connect identity provider** | Advertised by the discovery service's auth configuration. Storage Navigator redirects here for authorization code exchange, and back here for token refresh and logout, when authentication is enabled for the stack. |
| **Storage REST API** | The primary API surface: folder and file listing, uploads, downloads, tags, and versioning. |
| **Permission REST API** | Optional. When the discovered stack advertises a permission service, Storage Navigator surfaces permission management for the selected path. See {ref}`Permissions Integration <storage-navigator-app-permissions-permissions-integration>`. |
| **Thumbnail REST API** | Optional. When advertised, used to render file thumbnails in the content views. |

(storage-navigator-app-architecture-request-flow)=
## Request Flow

1. On page load, the browser fetches the static runtime configuration file once for the session. See {ref}`Configuration <storage-navigator-app-configuration-configuration>`.
2. Storage Navigator resolves a storage stack to connect to, using either the configuration's default discovery URL or a discovery URL the user enters manually, and queries that discovery service for the stack's advertised endpoints and auth configuration.
3. When the discovery-advertised auth configuration is present, the user is redirected to the identity provider's authorization endpoint using the OpenID Connect authorization code flow with PKCE. When no auth configuration is advertised, the stack is treated as unauthenticated and requests are sent without a bearer token.
4. Every REST request to a discovered service attaches the current access token as a bearer token, refreshing it proactively before expiry or reactively on a `401`/`403` response.
5. The resolved storage, permission, and thumbnail endpoints back the folder tree, content views, uploads, tags, versioning, and optional permission panel described in {ref}`Features <storage-navigator-app-features-features>`.

Steps 2 and 3 cover discovery resolution, the authorization code exchange, and token refresh; see {ref}`Authentication <storage-navigator-app-authentication-authentication>` for the full detail.

(storage-navigator-app-architecture-see-next)=
## See next

- {ref}`Authentication <storage-navigator-app-authentication-authentication>`. The discovery-to-authorization flow in detail.
