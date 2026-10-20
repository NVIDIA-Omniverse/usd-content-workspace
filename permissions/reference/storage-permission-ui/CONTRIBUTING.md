# Contributing

`@omniverse/storage-permission-ui` is developed by the NVIDIA Omniverse USD
Storage APIs team and published as part of the
[USD Content Workspace](https://github.com/NVIDIA-Omniverse/usd-content-workspace)
repository, under `permissions/reference/storage-permission-ui`. The published
copy is updated from the team's development repository on each release; it is
provided so that the permission panel can be read, built, and integrated.

The repository is currently not accepting external code contributions; see its
top-level [CONTRIBUTING.md](https://github.com/NVIDIA-Omniverse/usd-content-workspace/blob/main/CONTRIBUTING.md).
Edits made directly to this directory there are overwritten by the next release
snapshot.

If you have found a problem or want a change:

- For a **security vulnerability**, do not open a public issue. Follow
  [SECURITY.md](SECURITY.md) and report it to NVIDIA PSIRT.
- For a bug or a feature request, open an issue in the
  [USD Content Workspace](https://github.com/NVIDIA-Omniverse/usd-content-workspace/issues)
  repository and name `storage-permission-ui` in the title. Include the package
  version, the steps to reproduce, and what you expected to happen.

## Checks

Changes to this package keep the following passing:

- `npm run test` — runs the type check, the Prettier check, ESLint, and the
  Vitest suite.
- `npm run build:lib`.
- Every source file keeps its SPDX Apache-2.0 header.
