# permission-ui

Web UI for managing **services** and **policies** in the Permission Service backend. The app uses **OpenID Connect** for authentication and loads its runtime configuration from `GET /config/main.json`.

## Prerequisites

- **Node.js**: 22+
- **npm**: comes with Node.js

## Install

```bash
npm ci
```

## Local development

1) Create `public/config/main.json` (see below)

2) Start the dev server:

```bash
npm run dev
```

The app will be available at `http://localhost:3180`.

## Build

```bash
npm run build
```

### Container image

By default the `Dockerfile` serves the app from the public `nginxinc/nginx-unprivileged`
image. Its `public-nginx` stage gives that image the layout the Helm chart expects: nginx
reads `/usr/local/nginx/conf/nginx.conf`, serves `/var/www/html` on port 8080, and runs as
the non-root user `nvs` (uid 1000). NVIDIA builds the published image with
`--build-arg SERVE_IMAGE=<simple-nginx image>`, its own distroless nginx image with the
same layout. To serve from another nginx image, pass `--build-arg SERVE_IMAGE=<image>`;
the image must serve `/var/www/html` with the nginx config provided by the Helm chart.
Also pass `--build-arg SERVE_USER=<user>` with a non-root user that exists in your image.

## Test

```bash
npm test
```

## Runtime config: `public/config/main.json`

In development, Vite serves files from `public/`, so the app expects the config at `public/config/main.json` (and it will be available at `/config/main.json`).

This file is **git-ignored** (see `.gitignore`), so each developer must create it locally.

### Values

- **`auth.clientId`**: OpenID Connect client ID registered in your identity provider for this application.
- **`auth.authority`**: Identity provider authority/issuer base URL used by the OIDC client (for example a Keycloak realm URL).
- **`auth.redirectUri`**: Full URL where the identity provider redirects back after login. This app has a dedicated callback route at **`/openid`**.
- **`auth.scope`**: Space-separated scopes requested from the identity provider. If omitted, defaults to `openid profile email`.
- **`endpoints.backend`**: Base URL of the Permission Service backend. The UI calls endpoints under `/v1beta/...` relative to this base URL.
- **`endpoints.userService`** *(optional)*: Base URL of the User Info Service. When set, the UI resolves policy principal identifiers to human-readable display names and enables user autocomplete in the policy form. The UI calls endpoints under `/api/v1alpha/...` relative to this base URL.

### Example `public/config/main.json`

```json
{
  "auth": {
    "clientId": "permission-ui",
    "authority": "https://auth.example.com/realms/my-realm",
    "redirectUri": "http://localhost:3180/openid",
    "scope": "openid profile email"
  },
  "endpoints": {
    "backend": "http://localhost:8080",
    "userService": "http://localhost:8090"
  }
}
```

## Deploy (Helm)

This repo includes a Helm chart at `helm/permission-ui`. The chart renders a `main.json` ConfigMap entry and mounts it into the container at `/var/www/html/config/main.json` so the UI can fetch it as `/config/main.json`.
