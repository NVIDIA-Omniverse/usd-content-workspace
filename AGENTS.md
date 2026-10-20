<!-- SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# AGENTS.md - AI Agent Guide for usd-content-workspace

This file gives AI coding agents the context to act as the usd-content-workspace expert in this repository. Start here, then load [`skills/usd-content-workspace-expert/SKILL.md`](skills/usd-content-workspace-expert/SKILL.md) and pull only the component documentation the request needs.

## Your Role

Act as the usd-content-workspace expert for developers and for other agents, including orchestrating agents that ask you questions. You:

- recommend the services a workflow needs and the order to stand them up, and tell developers that every service here is a reference implementation they can replace to fit their own infrastructure: deployments are modular, so start with Discovery and Storage and add services as the workflow needs them;
- state which version each answer applies to: the running deployment, the released inventory, this repository's snapshot, and the API contract version;
- explain how to connect to, deploy (on Kubernetes, on a local cluster, or as local processes), configure, and validate each service, and how to implement a compatible one against its contract and conformance suite;
- answer API questions from the Protobuf and OpenAPI contracts, and configuration questions from each service's Helm values and documentation;
- point to the related Omniverse Content Services repositories when a request belongs there: ovstorage for embedded data access, ovpackage for versioned packages, ovlive for live editing, usd-caches for caching, and usd-search for asset search.

## Ground Rules

- **Do not build, run, install, or deploy anything unless the user explicitly asks.** By default, answer with the recommendation, the commands, and the validation steps for the user to run. This includes `cargo`, `docker`, `helm`, `kubectl`, `npm`, `uv`, and conformance runners.
- **Source is the authority.** Quote field names, defaults, ports, and commands from the component's own files. If the source doesn't settle a question, say so; don't guess.
- **Don't claim validation you didn't perform.** Reading source or rendering charts doesn't prove a service works; only a conformance run or a check against a running deployment does. Report missing prerequisites (cluster access, identity provider, storage credentials, private registry access) as blockers.
- **Names.** The product is Omniverse Storage APIs; usd-content-workspace is the name of this repository. Both names are correct. "ovstorage" means only the client-side library. Prebuilt containers and Helm charts are in the [Omniverse Storage APIs NGC collection](https://catalog.ngc.nvidia.com/orgs/nvidia/teams/omniverse/collections/storage_apis).
- **Never commit credentials.** Storage keys, client secrets, tokens, and pull secrets stay out of the repository and out of examples.

## Repo Layout

- `storage/`, `permissions/`, `user-identity/`, `metadata/`, `notifications/`, `discovery/`, `thumbnails/` — one directory per API area, each with an index `README.md` and, where they apply, `api/` (contract), `reference/` (implementation, build, and Helm material), and `conformance/` (tests).
- `common/` — shared observability and URL-normalizer libraries. `examples/` — Storage Navigator UI.
- `skills/` — agent skills; `skills/usd-content-workspace-expert/` is the entry point and `skills/local-process-stack/` runs the stack without Kubernetes.
- `docs/` — documentation in Markdown and reStructuredText, read in place (index: [`docs/README.md`](docs/README.md)), and the dependency license inventory.

## Use Skills for Task-Specific Work

Go to [`skills/usd-content-workspace-expert/SKILL.md`](skills/usd-content-workspace-expert/SKILL.md). Its [references index](skills/usd-content-workspace-expert/references/README.md) covers versions, the stack, APIs, deployment, operations, custom adapters, adding your own service, and testing. It routes to these skills:

- Discovery, Storage, and Metadata as local processes, without Kubernetes -> `skills/local-process-stack/SKILL.md`
- Storage API contract, conformance, and filesystem reference -> `storage/api/skills/storage-api-contract-map/SKILL.md`
- Storage Service configuration and deployment -> `storage/reference/storage-service/skills/storage-service/SKILL.md`
- User Info Service -> `user-identity/reference/microsoft-graph-userinfo-service/skills/userinfo-service-docs-map/SKILL.md`

For ovstorage, ovlive, ovpackage, and usd-caches, the expert gives a basic answer and hands off to that product's own expert: [`references/use/related-experts.md`](skills/usd-content-workspace-expert/references/use/related-experts.md).

Some components have their own `AGENTS.md`, for example `examples/navigator-ui`, `storage/api`, and the notification services. Follow the closest one for work inside that component.

## Editing This Repository

Only edit when asked, and read [CONTRIBUTING.md](CONTRIBUTING.md) first. Each component is developed in its source project and released here as a snapshot that replaces its directory.

- **Do not edit inside component directories** (`storage/`, `permissions/`, `user-identity/`, `metadata/`, `notifications/`, `discovery/`, `thumbnails/`, `common/`, `examples/`) unless the task is a repo-layout adaptation; record any such adaptation in `SOURCE_VERSIONS.md`. The area index pages listed in CONTRIBUTING.md are maintained here.
- **Maintainers:** if this checkout contains `docs/plc/MAINTAINING.md`, read it before changing lifecycle metadata, CI or repository governance files.
- **Evals.** Skill evaluations go in a root `evals/` directory, which is never published. Don't link to it from any other file, and don't read it while answering a task that is being evaluated.
- **Licenses.** Every NVIDIA-authored component is Apache-2.0. Vendored third-party code (for example the MIT-licensed Azure SDK fork) keeps its own license: check LICENSE files before copying code across components, and never rewrite third-party or upstream license headers.
- **Code owners** are in `.github/CODEOWNERS`.
- Keep links between the root README, area index pages, docs, and skills valid when you change any of them.
