# Doc Writing Guide — Thumbnails API

How to write and maintain documentation across the Thumbnails API (`thumbnail-service` Rust,
`thumbnail-task-queue` Python, `thumbnail-dev-utils` Bash). All project-level specs and quorum
artifacts live in `thumbnail-service`.

## Inline documentation

- **Rust:** use `///` rustdoc on public items — types, functions, and modules that other crates or
  transports depend on. Say what it does and any non-obvious invariant (e.g. "durations are humantime
  strings", "validated once at the boundary"), not what the signature already shows.
- **Python:** docstrings on public functions/classes; note side effects (uploads, DB writes,
  association callbacks) and the task-status transitions a Celery task drives.
- **Bash:** a header comment per script stating what it orchestrates, prerequisites, and its
  entry-point usage.
- Don't restate the code in a comment. Comment the *why* — the reason a non-obvious choice was made.

## When to update which doc

- **Architecture (`docs/sdd.md`)** — update when components, their responsibilities, external
  dependencies, or the data flow between the three repos change. Keep the component list and the flow
  diagram in sync with reality.
- **Requirements (`docs/prd.md`)** — update the relevant scope/acceptance section when user-visible
  capability changes; add a `### Feature:` subsection for new features (the workflow does this via
  `/quo-plan`). Never overwrite existing scope — the PRD is cumulative.
- **Customer-facing READMEs** — update the README of the **repo the change lives in** when
  user-visible behavior, endpoints, config, or deployment steps change. Put repo-specific content in
  that repo's README; keep shared API behavior described consistently across all three.
- **ADRs (`docs/adr/`)** — add a new numbered ADR for a significant, hard-to-reverse design decision;
  don't edit a decided ADR's rationale after the fact.

## Style

- Active voice, direct, present tense. "The worker uploads the thumbnail," not "the thumbnail will be
  uploaded by the worker."
- **Show code/commands over prose** where a snippet is clearer. Use fenced blocks with a language tag.
- Reference config by its exact env-var name (`STORAGE_GRPC_ENDPOINT`) and endpoints by their exact
  path (`POST /v1alpha/associations`).
- Use the project's terminology consistently: **thumbnail association** (the source↔thumbnail link),
  **spec / specification** (a processing type), **task** (an async generation job), **event consumer**
  (the durable storage-event subscription), **task queue** (the Python/Celery tier).
- Enum values are written UPPER_SNAKE_CASE to match the code.

## Formatting

- Markdown with clear `##` section headings; wrap prose at a sensible width for reviewable diffs.
- Prefer tables for enumerated options (endpoints, config vars, specs) over long bullet chains.
- Keep diagrams as ASCII in the SDD so they diff cleanly.

## What not to document

- Don't document generated code (protobuf, OpenAPI) — the proto/spec is the source of truth.
- Don't duplicate the full config reference in prose; point at `.env.example`.
- Don't write aspirational docs for unbuilt behavior; document what exists. USD/video thumbnailing is
  out of scope today — don't describe it as if it ships.
