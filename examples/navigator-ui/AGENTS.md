# Storage Navigator — agent / contributor map

React + TypeScript SPA for browsing cloud storage: folder tree, grid/table content, uploads, tags, versions, permissions (optional). Uses OpenID against a discovery service to reach storage REST APIs.

## Stack

- **Bundler / dev**: Vite 6 (`vite.config.mjs`), `@` → `src/` via `vite-tsconfig-paths`
- **UI**: Mantine 7, `mantine-contextmenu`, Tabler icons, Allotment (split panes)
- **Data**: TanStack Query (ky HTTP client), Zustand + Immer + `persist` (`localStorage` + superjson)
- **Tests**: Vitest + jsdom, Testing Library, MSW (`src/test-utils/msw.ts`)

## Entry points

| File | Role |
|------|------|
| `src/Main.tsx` | DOM mount |
| `src/App.tsx` | QueryClient, Mantine, context menu, modals, `<Router />` |
| `src/Router.tsx` | `react-router-dom` v7: `/openid`, `/:endpoint/path/:path`, `/` → `<Main />` |
| `src/pages/Main/Main.tsx` | Primary app shell / layout |
| `src/pages/OpenId/OpenId.tsx` | OAuth callback handling |

## `src/` layout (where to look first)

Put new code in the folder that matches the **kind** of module (React vs plain TS, global vs feature-local). The table below states that contract explicitly.

| Path | What this folder is for | Typical contents |
|------|-------------------------|------------------|
| `components/` | **React components** — all feature and shared UI (JSX, styles, small local state). | One folder per feature area: `PathTree`, `PathContent`, `PathDetails`, `DetailsPanel`, modals, notifications, upload dropzone; each often has `*.tsx`, `*.module.css`, `*.test.tsx`, `index.ts`. |
| `hooks/` | **Custom React hooks** — all `use*` hooks used by components (data, effects, handlers). Not arbitrary TS modules; if it isn’t a hook, it belongs in `services/` or `util/`. | TanStack Query usage (`useFolderContentQuery`, mutations), uploads, clipboard, context menus, navigation, storage subscriptions; colocated `*.test.tsx`. |
| `pages/` | **Route-level React components** — top-level screens tied to URLs, minimal logic beyond composing layout. | `Main`, `OpenId`, etc. |
| `providers/` | **React context providers** — wrapper components that supply context to a subtree (Mantine extensions, app-specific providers). **Not** “service providers” in the DI sense. | e.g. `ModalsProvider`; `App.tsx` composes providers mounted near the root. |
| `services/` | **Plain TypeScript** — HTTP, auth, parsing, side-effect modules **without** React. Consumed by hooks and sometimes by store setup. | **api.ts** (ky client factory, ky `hooks` for tokens — not React hooks). **auth.ts**, **discoveryService.ts** (Zod), **notificationService.ts**. **storage/** REST helpers: `fileObjectService`, `fileFolderService`, `metadataService`, `versioningService`, `capabilitiesService`, `index.ts`. |
| `state/` | **Global client state and types** — Zustand store, slices, cross-cutting path types. | **appStore.ts**, **path.ts**, **slices/** (`connectionSlice`, `pathTreeSlice`, `clipboardSlice`, `applicationSlice`). |
| `util/` | **Small shared utilities** — pure or generic helpers, no React tree, no app “feature” ownership. | `url`, `batch`, Zod schema helpers, `number`, etc. |
| `test-utils/` | **Test-only** — custom render, MSW, mocks shared across tests. | `render.tsx`, MSW setup, filesystem API mocks. |
| `stubs/` | **Optional dependency shims** for the bundler when a package is missing. | e.g. `@omniverse/storage-permission-ui` stub via `vite.config.mjs` alias. |
| `theme.ts` (file) | **Mantine theme object** at repo root of `src/`, imported by `App.tsx`. | Single config file, not a folder. |

## Backend / auth flow (high level)

1. User configures a **discovery** URL; `discoveryService` loads stack metadata and auth client config.
2. **api.ts** `create()` builds a ky instance per service URL; **auth.ts** handles tokens and refresh using discovery.
3. Storage calls go through `services/storage/*` from hooks/components.

## Conventions

- Imports use **`@/...`** for `src` (see `tsconfig` paths).
- New presentational modules: `npm run scaffold` (Plop) — templates under `plop-template/`.
- Co-locate tests: `*.test.ts` / `*.test.tsx` next to source; MSW for HTTP in tests.

## Commands (from repo root)

- `npm run dev` — Vite on port **8080**
- `npm run test` — typecheck, Prettier check, ESLint + Stylelint, Vitest (see `.cursor/rules` if present)

## Related docs

- Human setup and optional deps: **README.md**
