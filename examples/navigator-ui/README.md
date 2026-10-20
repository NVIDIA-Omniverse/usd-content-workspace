# Storage Navigator Sample

Storage Navigator is a React + TypeScript single-page application that demonstrates how to
browse and manage cloud storage through the Omniverse USD Storage REST APIs. It authenticates
with OpenID against a discovery service and provides a reference UI for folder trees,
grid/table content views, uploads, tags, versions, and optional permission management.

The source is published under
[`examples/navigator-ui`](https://github.com/NVIDIA-Omniverse/usd-content-workspace/tree/main/examples/navigator-ui)
in the USD Content Workspace repository as a filtered snapshot that leaves out CI configuration,
IDE and agent tooling, migration reports, and local overrides.

## Components

The sample is organized under `src/` (see [`AGENTS.md`](AGENTS.md) for the full map):

- **App shell & routing** (`App.tsx`, `Router.tsx`, `pages/`) – QueryClient, Mantine theme,
  context menus, modals, and route-level screens including the OpenID callback.
- **UI components** (`components/`) – feature and shared UI: `PathTree`, `PathContent`,
  `PathDetails`, `DetailsPanel`, modals, notifications, and the upload dropzone.
- **Hooks** (`hooks/`) – TanStack Query data hooks and mutations, uploads, clipboard,
  navigation, and storage subscriptions.
- **Services** (`services/`) – React-free TypeScript: the ky HTTP client factory (`api.ts`),
  `auth.ts`, `discoveryService.ts`, and `storage/*` REST helpers (files, folders, metadata,
  versioning, capabilities).
- **State** (`state/`) – Zustand store, slices, and shared path types.
- **Utilities** (`util/`) and **test utilities** (`test-utils/`, MSW mocks).
- **Deployment** – a `Dockerfile` and a Helm chart under `helm/storage-navigator` for
  containerized and Kubernetes deployments.

## Prerequisites

* [Node.JS 22+](https://nodejs.org/en/download/)

## Run

Use `npm run` command with the following parameters:

### Build and dev scripts

- `dev` – start development server
- `build` – build production version of the app
- `preview` – locally preview production build

### Testing scripts

- `typecheck` – checks TypeScript types
- `lint` – runs ESLint
- `prettier:check` – checks files with Prettier
- `prettier:write` – formats all files with Prettier
- `vitest` – runs vitest tests
- `vitest:watch` – runs vitest in watch mode
- `vitest:coverage` – runs vitest with coverage report
- `test` – runs `vitest`, `prettier:check`, `lint` and `typecheck` scripts
- `scaffold` - runs plop generator to scaffold a new React component. templates located in /plop-templates

### Installing optional dependencies

This application has an optional dependency:

- `@omniverse/storage-permission-ui` -- Adds a permission management UI in the right panel that integrates with the Permission Service.

In this repository it is installed from its sources in
[`permissions/reference/storage-permission-ui`](../../permissions/reference/storage-permission-ui/README.md)
(`.npmrc` sets `install-links=true`, so npm copies the package instead of symlinking it and the
host application's React and Mantine are shared with it). Build the library before installing
this application:

```bash
(cd ../../permissions/reference/storage-permission-ui && npm ci && npm run build:lib)
npm ci
```

The application builds and runs without it: when the package is not installed or its library
has not been built, the build substitutes the stub in `src/stubs/` (see `vite.config.mjs`).

## Run in k8s

### Set up a local cluster (optional)

If you don't have a Kubernetes cluster, you can run this example locally with the following dependencies:
* [Docker](https://docs.docker.com/engine/install/) with BuildKit.
    * If you need to install BuildKit manually, use the following script: https://gist.github.com/jniltinho/bcb28a99aef33dcb5f35c297bf71e4ae
* [kubectl](https://kubernetes.io/docs/tasks/tools/install-kubectl-linux/)
* [helm](https://helm.sh/docs/intro/install/)
* [k3d](https://k3d.io/)
* [NodeJS 22+](https://nodejs.org/en/download/package-manager)

Create a local k3d cluster with a local Docker registry and map the `traefik` ingress controller to port 8080:

   ```bash
   k3d cluster create storage-navigator --registry-create storage-navigator-registry:127.0.0.1:42607 --api-port 6550 -p "8080:443@loadbalancer"
   ```

### Run with helm

> **Note:** By default the `Dockerfile` serves the app from the public
> `nginxinc/nginx-unprivileged` image. Its `public-nginx` stage gives that image the layout
> the Helm chart expects: nginx reads `/usr/local/nginx/conf/nginx.conf`, serves
> `/var/www/html` on port 8080, and runs as the non-root user `nvs` (uid 1000). NVIDIA builds
> the published image with `--build-arg SERVE_IMAGE=<simple-nginx image>`, its own
> distroless nginx image with the same layout. To serve from another nginx image, pass
> `--build-arg SERVE_IMAGE=<image>`; the image must serve `/var/www/html` with the nginx
> config provided by the Helm chart. Also pass `--build-arg SERVE_USER=<user>` with a
> non-root user that exists in your image.

1. Build and push Docker images to a registry:
   
   ```bash
   docker build -t storage-navigator-registry:42607/storage-navigator/storage-navigator:latest
   docker push storage-navigator-registry:42607/storage-navigator/storage-navigator:latest
   ```

2. Create `values.local.yaml` file in `helm/storage-navigator` directory with the following content:
   
   ```yaml
   image:
     # Path to the Docker registry with the image
     repository: storage-navigator-registry:42607/storage-navigator/storage-navigator
     
     # Tag created during docker build.
     # Use "latest" only for development.
     tag: latest
   ```
   
3. Navigate to `helm/storage-navigator` directory and run `helm install` command:

   ```bash
   cd helm/storage-navigator
   helm install storage-navigator . -f values.yaml -f values.local.yaml
   ```

4. (Optional) Forward port 8080 using `kubectl` if you're using a local cluster:
   
   ```bash
   export POD_NAME=$(kubectl get pods --namespace default -l "app.kubernetes.io/name=storage-navigator,app.kubernetes.io/instance=storage-navigator" -o jsonpath="{.items[0].metadata.name}")
   export CONTAINER_PORT=$(kubectl get pod --namespace default $POD_NAME -o jsonpath="{.spec.containers[0].ports[0].containerPort}")
   echo "Visit http://127.0.0.1:8080 to use your application"
   kubectl --namespace default port-forward $POD_NAME 8080:$CONTAINER_PORT
   ```

## Integration with other repositories

Storage Navigator is a client of the Omniverse USD Storage platform and integrates with
several external repositories at runtime and build time:

- **Discovery service** – On startup the app loads a discovery URL that supplies stack
  metadata and OpenID client configuration (`services/discoveryService.ts`, `services/auth.ts`).
  Point the app at your environment's discovery endpoint to reach the storage REST APIs.
- **Storage REST APIs** – `services/storage/*` calls the storage endpoints resolved from
  discovery.
- **Permission Service UI** – The optional `@omniverse/storage-permission-ui` package adds a
  permission management panel that talks to the Permission Service. Build and install it from
  `permissions/reference/storage-permission-ui` as described in [Installing optional dependencies](#installing-optional-dependencies); a build
  stub keeps the app compiling when the package is absent.

To embed the built app into another deployment, build the Docker image and deploy the Helm
chart under `helm/storage-navigator`, overriding the discovery URL and image repository for
your environment (see [Run in k8s](#run-in-k8s)).

## Libraries

This example application uses the following libraries:

| Use                          | Library                                                                                                                        |
|------------------------------|--------------------------------------------------------------------------------------------------------------------------------|
| Build Tool                   | [Vite](https://vitejs.dev/)                                                                                                    |
| Language                     | [TypeScript](https://www.typescriptlang.org/)                                                                                  |
| UI Component Library         | [Mantine](https://mantine.dev/)                                                                                                |
| Iconography                  | [Tabler Icons](https://tabler.io/icons)                                                                                        |
| Data Fetching / Server State | [TanStack Query](https://tanstack.com/query)                                                                                   |
| Client State                 | [Zustand](https://zustand-demo.pmnd.rs/)                                                                                       |
| Client Routes                | [React Router](https://reactrouter.com/en/main)                                                                                |
| CSS                          | [PostCSS](https://postcss.org/) with [mantine-postcss-preset](https://mantine.dev/styles/postcss-preset)                       |
| UI Prototyping               | [Storybook](https://storybook.js.org/)                                                                                         |
| Testing                      | [Vitest](https://vitest.dev/) setup with [React Testing Library](https://testing-library.com/docs/react-testing-library/intro) |
| Static Analysis              | ESLint setup with [eslint-config-mantine](https://github.com/mantinedev/eslint-config-mantine)                                 |