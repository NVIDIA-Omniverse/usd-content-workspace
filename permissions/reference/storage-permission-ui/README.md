# Storage Permission UI

`@omniverse/storage-permission-ui` is a React component library that renders the
permission management panel for the Omniverse Storage Permission Service. It
lets a host application display and edit the access rules for a storage resource
(an object or folder) without re-implementing the permission workflows.

## What it provides

The library exposes a single top-level component and the types needed to
configure it:

- `PermissionForm` — the panel component. Given authentication tokens, service
  configuration, and a target resource, it loads and renders:
  - **Assigned** permissions: the rules attached directly to the selected
    resource, editable inline.
  - **Resolved** permissions: the effective access for the resource, including
    group-inherited rules when a user information service is configured.
  - Warnings when global policies affect the resource but cannot be shown or
    managed in the panel.
- `PermissionFormProps` — props accepted by `PermissionForm`.
- `PermissionAuth` — access and ID tokens used to call the services.
- `PermissionConfig` — base URLs of the permission service and the optional user
  information service.
- `ResourceAddress` — the resource type (`object` or `folder`) and its address.

Internally the library talks to the permission service through a small API layer
(`src/api`) and resolves principal and group names through the optional user
information service (`src/hooks`).

## Requirements

The library ships as ESM and relies on the host application to provide the
following peer dependencies (see `peerDependencies` in `package.json`):

- `react` and `react-dom` (v19+)
- `@mantine/core`, `@mantine/form`, `@mantine/hooks`, `@mantine/notifications`
- `@tanstack/react-query`
- `@tabler/icons-react`
- `zod`

## Building

The package is published to the Omniverse npm registry, so an
authenticated `.npmrc` (see `.npmrc.ci`) is required to install dependencies.

```bash
npm ci            # install dependencies
npm run build:lib # emit the library into dist/ via tsconfig.lib.json
```

Additional scripts:

- `npm run dev` — run the local example app (`src/examples/Example.tsx`) with
  Vite for manual testing against a running service.
- `npm run build:example` — type-check and build the standalone example bundle.
- `npm run test` — the full CI check: `typecheck`, `prettier`, `lint`, and
  `vitest`.

## Integration

Install the package in the host application:

```bash
npm install @omniverse/storage-permission-ui
```

`PermissionForm` expects a Mantine theme provider and a React Query client in
the surrounding tree. Provide them once near the root of the host application:

```tsx
import { MantineProvider } from "@mantine/core";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { PermissionForm } from "@omniverse/storage-permission-ui";

const queryClient = new QueryClient();

function StoragePermissions() {
  return (
    <MantineProvider>
      <QueryClientProvider client={queryClient}>
        <PermissionForm
          auth={{ accessToken, idToken }}
          config={{
            url: "https://permissions.example.com",
            userInfoUrl: "https://user-service.example.com",
          }}
          resource={{ type: "object", address: resourceUrl }}
        />
      </QueryClientProvider>
    </MantineProvider>
  );
}
```

Notes:

- `config.url` is the base URL of the permission service that backs the panel.
- `config.userInfoUrl` is optional. When omitted, the current user's identity is
  resolved from the ID token and group-inherited permissions are not shown.
- `auth.accessToken` and `auth.idToken` must be valid tokens accepted by the
  configured services; the host is responsible for obtaining and refreshing
  them.

See `src/examples/Example.tsx` for a complete, runnable example that wires the
form up to a configurable service endpoint.
