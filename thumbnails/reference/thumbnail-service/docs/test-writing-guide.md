# Test Writing Guide — Thumbnails API

Conventions for writing tests across the Thumbnails API. Rust (`thumbnail-service`) is the primary
target; Python (`thumbnail-task-queue`) notes are called out where they differ.

## Organization

- **Rust unit tests** live next to the code they cover — `thumbnail-common` keeps its suite under
  `crates/common/src/tests/` and inline `#[cfg(test)] mod tests` blocks. Test the business logic in
  `common`; transport crates test mapping/validation, not re-tested business rules.
- **Integration tests** that exercise a running service or the dev cluster are separate from unit
  tests; the authoritative integration run is against the dev cluster defined in the
  `usd-storage-api-deployment` repository (`environments/dev/`).
- **Python:** `pytest` under the task-queue repo's test tree; keep unit tests (image transforms,
  schema validation, status mapping) independent of a live broker/DB.

## Naming

- Name tests for the behavior under test and the condition:
  `create_thumbnails_rejects_unsupported_spec`, not `test1`. The name should read as the assertion.
- One clearly-named behavior per test; if you need "and" in the name, split it.

## Structure

- **Arrange-Act-Assert.** Build inputs, perform the one action, assert the outcome. Keep the acted-on
  call obvious.
- Use named constants for magic values (dimensions, timeouts, ids) so the intent is legible.
- Prefer small helper constructors for test data over copy-pasted struct literals.

## Async tests

- **Rust:** use `#[tokio::test]` for async paths. Use `serial_test`'s `#[serial]` for tests that
  touch shared/global state (env, singletons) so they don't race.
- Never `sleep` to synchronize — await the actual signal/future. Put a timeout around anything that
  could hang so a broken test fails fast instead of blocking CI.
- **Python:** use `pytest`'s async support for coroutine paths; don't block the event loop.

## Mocking & isolation

- **Prefer hand-written trait mocks / fakes over heavy frameworks.** For outbound HTTP (task queue),
  use `wiremock` to stand up a controlled endpoint. For gRPC/storage, inject a fake implementing the
  client trait rather than reaching the network.
- Each test is self-contained: no ordering dependencies, no shared mutable fixtures that leak between
  tests. A test must pass when run alone and in the full suite.
- **Python:** fake the storage gRPC client and the association callback; don't require a live
  RabbitMQ/Postgres for unit tests.

## Coverage expectations

- Cover the **happy path, the error paths, and boundaries** — not just the success case. For each new
  branch (validation failure, dependency error, retry exhaustion), add a test.
- **Property-based tests** (`proptest`) are already used in `thumbnail-common`; reach for them on
  parsing/conversion logic (spec parsing, enum round-trips, size calculations) where the input space
  is large.
- Assert on **observable behavior**, not internal implementation details — a refactor that preserves
  behavior should not break the test.

## What not to test

- Don't test generated code (protobuf stubs, OpenAPI) or third-party libraries.
- Don't assert exact log strings or private helper internals.
- Don't write tests that only restate the implementation; if it can't fail, it isn't earning its
  place.

## Running

- **Narrow (while iterating):** `cargo test -p thumbnail-common` (or the crate you're editing);
  `pytest <path>` for Python.
- **Full (before done):** `cargo build --workspace`; `cargo test --workspace`;
  `cargo clippy --all-targets --all-features -- -D warnings`; `pytest` for the task queue.
  `make ci-check` mirrors CI locally for `thumbnail-service`.
