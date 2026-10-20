# Test Review Guide — Thumbnails API

A checklist for reviewing tests across the Thumbnails API (`thumbnail-service` Rust primary;
`thumbnail-task-queue` Python). Flag anything that fails these checks.

## Correctness

- [ ] Tests assert **observable behavior**, not implementation details — a behavior-preserving
      refactor wouldn't break them.
- [ ] Assertions are specific: exact expected values, not just "is not null" / "did not panic".
- [ ] The test actually exercises the code under test (not a mock returning the asserted value —
      no tautologies).
- [ ] Enum-value assertions use the UPPER_SNAKE_CASE canonical form consistently.

## Isolation

- [ ] No cross-test ordering dependencies; each test passes alone and in the full suite.
- [ ] Global/shared state (env vars, singletons, cached channels) is guarded with `#[serial]` or
      reset between tests.
- [ ] No reliance on a live external service for unit tests — storage/task-queue/broker/DB are faked
      or `wiremock`ed.

## Coverage

- [ ] Happy path **and** error paths **and** boundaries are covered for new/changed branches.
- [ ] New validation rules, dependency-failure handling, and retry-exhaustion paths each have a test.
- [ ] Parsing/conversion logic with a large input space uses `proptest` where practical.

## Robustness

- [ ] No `sleep`-based synchronization — tests await the real signal/future.
- [ ] Anything that could hang has a timeout so failures surface fast.
- [ ] Async tests use `#[tokio::test]` (Rust) / proper async fixtures (Python) rather than blocking.

## Readability

- [ ] Arrange-Act-Assert structure is clear; the acted-on call is obvious.
- [ ] Test names describe behavior + condition and read as the assertion.
- [ ] Magic values are named constants; test data is built via helpers, not sprawling literals.

## Performance

- [ ] Tests are fast — no unnecessary network, no large real image files where a small fixture works.
- [ ] Shared expensive setup is scoped appropriately without introducing cross-test coupling.

## Anti-patterns to flag

- Tests that can never fail (asserting a constant, or asserting a mock's own return).
- Over-mocking that tests the mock rather than the code.
- Snapshot/log-string assertions that break on cosmetic changes.
- Tests of generated code or third-party libraries.
- Copy-pasted tests that differ only in one value where a table/parameterized test is clearer.
