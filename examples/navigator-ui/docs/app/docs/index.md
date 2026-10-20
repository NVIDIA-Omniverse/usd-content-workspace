(storage-navigator-app-index-omniverse-storage-navigator)=
# Omniverse Storage Navigator

(storage-navigator-app-index-overview)=
## Overview

Storage Navigator is a browser-based reference application for the Omniverse USD Storage APIs. It gives an operator or integrator a working, deployable example of a client built against the storage, permission, and thumbnail service contracts: a folder tree, grid and table content views, uploads, tags, versioning, and optional permission management, all driven entirely through published REST APIs reached via a discovery service.

Storage Navigator is a **reference client**, not a service that implements or serves any part of the API contract. It has no server-side logic of its own beyond serving a static single-page application; every capability it exposes is a call to a storage stack discovered at runtime. Read this documentation as a guide to deploying and configuring that client, and as a worked example of how a browser application authenticates against, discovers, and calls an Omniverse Storage API stack.

(storage-navigator-app-index-table-of-contents)=
## Table of Contents

- {ref}`Architecture <storage-navigator-app-architecture-architecture>`
- {ref}`Authentication <storage-navigator-app-authentication-authentication>`
- {ref}`Deployment <storage-navigator-app-deployment-deployment>`
- {ref}`Configuration <storage-navigator-app-configuration-configuration>`
- {ref}`Features <storage-navigator-app-features-features>`
- {ref}`Permissions Integration <storage-navigator-app-permissions-permissions-integration>`
- {ref}`Troubleshooting <storage-navigator-app-troubleshooting-troubleshooting>`

(storage-navigator-app-index-architecture-summary)=
## Architecture Summary

Storage Navigator is a single-page application served as static assets behind an nginx container. It has no backend of its own; at startup, it loads a static runtime configuration file and then talks directly to a storage API stack over REST.

| Area | Overview |
|---|---|
| Deployment unit | A single nginx container serving a pre-built static bundle, deployed with the Storage Navigator Helm chart |
| Client stack | A browser single-page application; no server-side application logic |
| APIs consumed | Storage, permission, and thumbnail REST APIs, reached through a discovery service |
| Authentication | OpenID Connect authorization code flow with PKCE, driven by discovery-advertised identity provider configuration |
| Runtime configuration | A static `config.json` file, mounted into the container and fetched once per session |

At a high level, Storage Navigator:

1. Loads its runtime configuration (`config.json`) once when the page boots.
2. Resolves one or more storage stacks through a discovery service, using either the operator-configured default stack or a stack the user adds manually.
3. Reads the discovery-advertised auth configuration and, when authentication is enabled, redirects the user through the identity provider's OpenID Connect authorization code flow.
4. Calls the storage, permission, and thumbnail REST APIs discovered for that stack, attaching the resulting bearer token to every request.

For the full authentication flow and the discovery-then-authorize sequence, see {ref}`Authentication <storage-navigator-app-authentication-authentication>`. For the deployment unit and the container's runtime configuration surface, see {ref}`Deployment <storage-navigator-app-deployment-deployment>` and {ref}`Configuration <storage-navigator-app-configuration-configuration>`.

```{toctree}
:maxdepth: 1
:caption: Contents:
architecture
authentication
deployment
configuration
features
permissions
troubleshooting
```

(storage-navigator-app-index-see-next)=
## See next

- {ref}`Architecture <storage-navigator-app-architecture-architecture>`. The client stack and the discovery-to-storage-call data flow.
