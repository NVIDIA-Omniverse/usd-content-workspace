# Engineering Best Practices — Thumbnails API

Opinionated defaults for the Thumbnails API. The API spans three repos: **`thumbnail-service`**
(Rust — the primary surface these practices target), **`thumbnail-task-queue`** (Python/Celery), and
**`thumbnail-dev-utils`** (Bash). Where a rule is language-specific it's labelled; otherwise it
applies everywhere.

## Module & crate boundaries (Rust)

- **Shared logic goes in `thumbnail-common`.** Anything both the HTTP and gRPC transports need lives
  in the `common` crate. Transport crates (`thumbnail-http`, `thumbnail-grpc`) do request/response
  mapping and nothing more — no business rules, no outbound calls duplicated per transport.
- Keep outbound-service clients (storage, task queue, notifications) in their `common` submodules;
  don't reach around them from a transport crate.
- Generated code (protobuf stubs, OpenAPI) is regenerated via `scripts/buf-gen.sh` and
  `scripts/generate-openapi.sh` **only when an interface changes** — never hand-edit generated files.
  For the proto crates that is not a style preference: the generator overwrites
  `Cargo.toml`, `lib.rs`, and `src/**` wholesale, so an edit there is gone on the next run.
  Crate-level configuration belongs in the checked-in `Cargo.toml.template` instead, and
  consumers import fully-qualified `nvidia::omniverse::…` paths rather than crate-root
  aliases that regeneration would delete. See "Regenerating protobuf code" in `CLAUDE.md`.

## Configuration

- **All config is env-var / `clap`-driven** via the typed `RuntimeConfigArgs` groups in
  `crates/common/src/config.rs`. Add new config as a typed field, not an ad-hoc `std::env::var` read.
- **Fail fast at startup.** Required config that's missing or malformed must abort startup with a
  clear error — never limp along with a default that hides a misconfiguration.
- **Durations are humantime strings** (`15s`, `2m`), parsed via `humantime` — do not add integer
  `*_MS` / `*_SECONDS` fields.
- Document every new variable in `.env.example`.
- Python (`thumbnail-task-queue`): config flows through `pydantic-settings`; keep the same fail-fast
  posture and add new settings as typed fields.

## Error handling

- **Rust: typed errors via `thiserror`.** Model failure modes as enum variants and match on them;
  never `string.contains("not found")` to drive control flow. Map domain errors to transport
  responses at the transport boundary.
- Use the shared retry helpers rather than open-coding retry loops; keep backoff/jitter consistent.
- **Python:** raise/return typed exceptions; let Celery's `max_retries` + `task_acks_late` handle
  transient failures rather than bespoke retry code inside a task. Record terminal failures as
  `FAILURE` task status, not swallowed exceptions.

## Async & concurrency

- **Rust:** `tokio` throughout. Don't block the async runtime with sync I/O or CPU-bound loops; use
  the appropriate async client. Reuse long-lived channels (e.g. the storage gRPC channel cached in a
  `OnceLock`) rather than reconnecting per call. Guard shared mutable state with the narrowest lock
  scope possible.
- **Python:** CPU-bound image work belongs in the Celery worker, not the FastAPI request path. Keep
  gRPC channels pooled; don't open a fresh channel per request.

## Type & API design

- **Enum values are UPPER_SNAKE_CASE** across proto, Rust, and the HTTP API — no exceptions. When you
  add an enum value, add it consistently in all layers.
- Mandatory-auth operations take `&str` (validated once at the boundary), not `Option<&str>` threaded
  through the call stack.
- Prefer parsing into domain types at the edge over passing raw strings/JSON deep into the code.
- API changes are versioned under `v1alpha`; keep the proto and the OpenAPI spec the source of truth
  and regenerate rather than divergently hand-editing.

## Observability

- **Rust:** use `tracing` spans/events, not `println!`. Include the relevant identifiers (task id,
  resource identity) in structured fields so events are filterable.
- **Python:** structured logging; surface Celery task transitions and gRPC failures with enough
  context to correlate a task across services.

## Storage & external services

- Treat the storage service as the authority for file/thumbnail metadata; `thumbnail-service` is the
  authority for associations. Don't write associations from the task queue directly — delegate back
  to `thumbnail-service`.
- Set connect/request timeouts and keepalive on outbound channels; a hung dependency must not hang the
  service.

## Security

- Never log secrets or tokens. Service-account credentials come from env/secret mounts; the
  access-token-file path is polled, not embedded.
- `DEVELOPMENT_MODE=true` disabling auth is **local-only** — never rely on it in a deployed config.
- Keep runtime images distroless and non-root (as the Dockerfiles already do); don't add tools to the
  runtime stage for convenience.

## Code style & dependencies

- **Rust:** code must pass `cargo fmt --all -- --check` and
  `cargo clippy --all-targets --all-features -- -D warnings`. Clippy warnings are errors — fix them,
  don't `#[allow]` without a written reason.
- SPDX/NVIDIA proprietary license headers stay on source and config files.
- Add dependencies deliberately; prefer the crate/library already in the workspace. Internal crates
  (`permission-auth`) and base images come from NVIDIA registries — don't substitute public
  equivalents.
- Run `make ci-check` (or the per-repo equivalent) before considering a change done; leave nothing in
  a non-compiling state.
