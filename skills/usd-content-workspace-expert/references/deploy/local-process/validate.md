# Validate a local process deployment

> **Applies to:** any local process deployment from [../local-process.md](../local-process.md) · **Released:** not a released mode · **This repo's snapshot:** Storage API conformance in unreleased Storage API source (after 1.0.0-beta.4); Discovery operator conformance · **Sources:** [`storage/api/conformance_tests/README.md`](../../../../../storage/api/conformance_tests/README.md), operator [`conformance/`](../../../../../discovery/reference/discovery-service/operator/conformance/), [Discovery Catalog API](../../../../../discovery/reference/discovery-service/operator/docs/api/docs/discovery-catalog-api.md)

Back to [local process deployment](../local-process.md). Use disposable configuration, mounts, and databases for every test that writes. Record the build commit, executable hash, and the addresses tested.

## 1. Discovery

- `GET /api/v2/services` returns `schema-version: 2`; if served, `GET /api/v1/services` returns `schema-version: 1`. Neither path returns the other's shape.
- Every advertised URL is reachable from the client's network location, including loopback and, if requested, the LAN address. No advertised service is absent; optional services that are not running are not listed.
- Discovery works with no token in unauthenticated local mode.
- An independent client (curl, Navigator, Kit with `OMNI_STORAGE_DISCOVERY`) finds Storage and Metadata from the catalog alone.

## 2. Contract conformance

Run the Storage API conformance suite against the storage endpoints; select native metadata scenarios as needed (`run-conformance-tests -k "native_metadata"`). Setup and options: [`conformance_tests/README.md`](../../../../../storage/api/conformance_tests/README.md) and [../../test/conformance.md](../../test/conformance.md). Record which tests were skipped as unsupported and confirm each skip matches a method the implementation intentionally leaves `UNIMPLEMENTED`.

## 3. Storage behavior beyond the suite

For a single-host build with a managed filesystem:

- Concurrent unconditional writes to one file keep every version, ordered by commit. Two files number versions independently. Competing conditional writes with the same previous identity produce exactly one success.
- Identical bytes written under two names and several versions are stored once per mount: check the content store, not only the reported digests. Reads return identical bytes. Deleting references one by one reclaims the content only after the last reference goes.
- Deleting a historical version, the current version, and the whole path each behave as specified (current deletion hides the file; history remains readable explicitly).
- Copy and move within a mount and across mounts or providers behave as documented; S3 versioning behavior is reported honestly, with unsupported conditions returned as unsupported.

## 4. Metadata

- Write and read through both metadata services, over REST and gRPC; values written through one are visible through the other.
- Path and version metadata stay independent; namespaces stay isolated; conditional etag updates race correctly.
- Stop the test process abruptly and restart: acknowledged metadata is still there.

## 5. Clients

Open, edit, and save a USD file from Kit through Discovery, and browse with Navigator. See [../../use/usd-workflows.md](../../use/usd-workflows.md).

## Reporting

Report what was observed, not inferred. Keep S3 fixture results separate from real cloud tests, and list the platforms actually tested. After switching to the real instance, run only read-only checks against saved content.
