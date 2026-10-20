---
name: storage-api-contract-map
description: Find the right Storage API material fast and act on it. Use when asked to implement a storage backend against the Omniverse Storage API, verify an implementation with the conformance suite, call the API from a client, stand up the reference filesystem service, or work out what changed between API versions. Routes to the authoritative files in this repo rather than restating them.
---

# storage-api-contract-map — the storage-protos contract map

## Scope of this skill

Everything this skill routes to is in this repo. The contract is defined here; implementations of
it are not, and this skill does not describe them.

The only implementation in scope is the one that ships here: `examples/filesystem_server/` — the
reference service, complete and versioned, which passes the conformance suite. Read it as an
illustration of the contract, not as the production system.

How any *particular* implementation is built, configured, deployed or scaled belongs to that
implementation's own documentation. If a question cannot be answered from the `.proto`, OpenAPI and
`.feature` files and the documents listed below, it is out of scope here.

**This skill is a table of contents, not a copy of the documentation.** It exists so an agent can
find the authoritative file in one hop, then read it. The contract is defined by the `.proto`,
OpenAPI and `.feature` files — not by prose about them.

`storage-protos` is the **published contract** for the Omniverse Storage API plus the means to
verify an implementation against it. It ships no production service: the services here are a
runnable reference and test fixtures.

## What defines the contract

The contract has two halves, and both are normative:

- **Shape** — the `.proto` files under `proto/nvidia/omniverse/storage/` and the OpenAPI specs
  under `openapi/`. These define messages, fields, endpoints and status codes.
- **Behaviour** — the Gherkin feature files under
  `conformance_tests/src/conformance_tests/features/<group>/<version>/*.feature`. **These are the
  source of truth for service behaviour**: ordering, idempotency, precondition handling, which
  error a given bad request must produce, and what a client is allowed to rely on. The step
  definitions that execute them are in `conformance_tests/src/conformance_tests/steps/`.

Where a behaviour is **not** specified by the `.proto`, OpenAPI or `.feature` files — for example,
the reaction to a particular kind of invalid input — the implementation is free to decide for
itself, and a client must not depend on what any one implementation happens to do. Behaviour stated
in a proto or OpenAPI comment is normative even where no scenario exercises it. If you need to pin
such a behaviour, change the relevant contract and add a scenario in a `.feature` file so the
conformance suite verifies it.

## Where the docs are

| Document | Path |
|---|---|
| Concepts, resource addressing and identity | `docs/user_manual.md` |
| Building a service (REST and gRPC walkthroughs) | `docs/service_example.md` |
| Calling the API (discovery, REST, gRPC) | `docs/client_example.md` |
| What a conformant implementation must satisfy | `conformance_tests/conformance_test_suite.md` |
| Running the suite against *your* service | `conformance_tests/README.md` |
| Reference service usage | `examples/filesystem_server/README.md` |
| Per-release change log | `CHANGELOG.md` |
| Repo build/test/lint targets | `README.md`, `Makefile` |
| Implementer guide shipped in the OSS bundle | `oss/AGENTS.md` |
| Rendered HTML of the above | `make user_docs`, output under `_build/docs/` |

`docs/index.rst` is the table of contents for the rendered docs; `docs/changes.rst` only includes
`CHANGELOG.md`, so read the changelog directly. `oss/AGENTS.md` paths are **bundle**-relative
(`filesystem_example/`, `conformance_tests/`), not repo-relative — in this repo the reference
service lives at `examples/filesystem_server/`.

### If you are reading this from the OSS distribution

The paths above are those of the source repository. The OSS distribution (built by `make oss`)
ships a subset under different names, so translate before you go looking:

| Source repository | OSS distribution |
|---|---|
| `proto/`, `openapi/` | same paths, supported versions only |
| `examples/filesystem_server/` | `filesystem_example/` |
| `conformance_tests/README.md`, `conformance_tests/src/` | same paths |
| `oss/AGENTS.md`, `oss/PROMPTS.md`, `oss/README.md` | `AGENTS.md`, `PROMPTS.md`, `README.md` |
| `oss/templates/` | `templates/` |
| `examples/python_client/`, `examples/python_service/` | same paths (included by the docs) |
| `CHANGELOG.md` | same path |
| `docs/*.rst`, `docs/*.md` | same paths (sources, not rendered) |
| `make` targets for the test runs | `run_tests.sh` |

Not shipped in the distribution: `Makefile`, `tools/`, `bootstrap.sh`,
`conformance_tests/conformance_test_suite.md`, `docs/rdme/`. Sections below that refer to those
are for work in the source repository.

## Task: implement a service against this contract

1. Read `docs/user_manual.md` for the addressing model, then `docs/service_example.md`.
2. Pick the version you are targeting from the table below, and read those `.proto` / `-api.yaml`
   files.
3. Read the `.feature` files for the behaviour, including error cases:
   `conformance_tests/src/conformance_tests/features/<group>/<version>/*.feature`.
4. Copy the shape of the reference service: `examples/filesystem_server/src/local_filesystem_service/`
   (`grpc_service/`, `rest_service/`, `backends/`, `filesystem/`). If your backend is Python, the
   fastest path is a new backend behind `backends/storage_backend_interface.py`.
5. Verify with the conformance suite (next section).

### The contract, by API group

Both a gRPC and a REST definition per group, under matching version directory names. The groups
are **not** all at the same version — check before assuming a version exists.

| Group | Proto | OpenAPI | Versions |
|---|---|---|---|
| Object read/write, copy, move, multipart | `proto/nvidia/omniverse/storage/fileobject/` | `openapi/fileobject/` | `v1alpha`, `v1beta` |
| Folder listing and enumeration | `proto/nvidia/omniverse/storage/filefolder/` | `openapi/filefolder/` | `v1alpha`, `v1beta` |
| Versions of an object | `proto/nvidia/omniverse/storage/versioning/` | `openapi/versioning/` | `v1alpha`, `v1beta` |
| User metadata | `proto/nvidia/omniverse/storage/metadata/` | `openapi/metadata/` | `v1alpha` only |
| Route discovery and capability queries | `proto/nvidia/omniverse/storage/capabilities/` | `openapi/capabilities/` | `v1alpha`, `v1beta` |

The `v1alpha` metadata surface shipped here is the supported one, and it is still being changed
here — `CHANGELOG.md` is the record of what the Metadata API currently offers. If that surface is
ever split out of this repository, record the new location in `CHANGELOG.md` and link it from here;
do not route metadata work elsewhere on the strength of this skill alone.

Version directories whose name contains `test` — `openapi/fileobject/v1testcopy`,
`openapi/fileobject/v1testmove`, `openapi/metadata/v1testmetadata` — declare **experimental** APIs.
They are not guaranteed to survive and can be phased out at any time, so do not build against them.
Each of these has already been promoted into `v1alpha`, which is where the supported surface lives.
They exist in the source repository only; the OSS distribution ships the supported versions alone.

## Task: verify an implementation

The suite is Gherkin over `pytest-bdd` and runs blackbox against **any** service that exposes the
endpoints — it is not tied to the reference service.

**`conformance_tests/README.md` is the instruction set for running it. Read that file and follow
it.** It is the authority on installation, pointing the suite at your own service, every
environment variable, supplying your own test data generator, the optional-mode runs and the
optional OpenAPI comparison. Do not work from a summary of it.

Two things it does not cover, both repo-root:

- From a fresh source checkout you must generate the ignored Python protobuf stubs first — see
  “Generate Python protobuf stubs without Packman” in `README.md`. A prepared wheel already
  includes them.
- To run the in-repo suite against the reference service, use the `make` targets documented in
  `README.md` (`conformance_test`, `blackbox_test`, `conformance_test_unversioned`,
  `conformance_test_readonly`, `blackbox_test_with_docker`); see the `Makefile` for the exact
  environment each sets.

Scope of the suite, from its own README: a consistency contract so generic clients can rely on
behaviour — not a quality or performance benchmark.

## Task: call the API from a client

- `docs/client_example.md` — discovery first, then REST and gRPC.
- `examples/python_client/` — runnable REST and gRPC scripts (`rest_upload.py`, `grpc_list.py`, …).
- `examples/python_service/` — minimal single-file REST and gRPC service skeletons.
- Start something to talk to: `local-filesystem-service filesystem --static-dir /tmp/storage`
  (REST `http://localhost:8011`, gRPC `localhost:50051`) — see `examples/filesystem_server/README.md`.

## Task: work out what changed

- `CHANGELOG.md` — the per-release, customer-facing change log. It ships in the OSS bundle, and it
  is the one to cite.
- `git log -- proto/... openapi/...` for the exact wire change, and the `.feature` files for the
  behaviour that changed with it.

## What this component needs from an environment

There is **no infrastructure to deploy here** — no database, no cluster, no credentials. What it
needs is a toolchain:

- **Python 3.10–3.12**, and **Poetry** for the two standalone packages (`conformance_tests/`,
  `examples/filesystem_server/`). For a fresh source checkout, first generate and copy the ignored
  `src/nvidia` protobuf files for **both** packages; `README.md` gives the no-Packman commands.
  A prepared wheel includes them. This path works without NVIDIA-internal access and is the one to
  use when you only want to run the suite against someone's service.
- **`bootstrap.sh` / `bootstrap.bat`** for full repo development. It uses **packman**
  (`tools/packman`, `deps/packman.deps.xml`) to fetch a pinned Python 3.10.13 into `venv/` and
  `oasdiff` into `.build/oasdiff/`, so it needs access to NVIDIA's packman repositories. Without
  packman, fall back to the Poetry path above and install `oasdiff` yourself.
- **`protoc`** via `grpc_tools` (`make protoc`) to generate Python stubs after bootstrapping;
  `make sync-conformance-test-nvidia sync-filesystem-example-nvidia` copies them into both packages.
  For a fresh checkout without Packman, use the commands in `README.md` before Poetry installation.
- **Docker** for `./tools/lint` and `./tools/format` (buf runs in a container) and for the
  containerised blackbox run.
- **Node/npm** only for the OpenAPI bundling targets (`@redocly/cli`, `rdme`).

Known non-working commands — do not try to debug them:

- **`make gov`** invokes `tools/spectral/local_gov.bat`, a Windows batch file that downloads
  NVIDIA API governance rulesets that are not publicly distributed. It does not run on Linux, and
  **`make lint` depends on it**, so `make lint` fails too — run `make mypy` directly. OpenAPI
  governance runs in CI, not locally.
- **`examples/filesystem_server/run_filesystem_service_test.sh`** is stale: it compiles a
  `metadata/v1testmetadata` proto that no longer exists and launches a `main.py` that was replaced
  by the `local_filesystem_service` module entry point. `conformance_test_suite.md` still points at
  it under an older name. Use `./run_conformance_test.sh` at the repo root instead.

## Conventions when changing the contract

These apply to work in the source repository; the OSS distribution is a build output of it.

- The default branch is **`trunk`**, not `main`.
- `buf lint` and `buf format` gate the proto surface; run them locally with `./tools/lint` and
  `./tools/format --diff` (`--write` to apply).
- Breaking-change detection runs on merge requests but is currently **advisory only** — it is
  configured to pass regardless of result. Do not read a green pipeline as proof you did not break
  consumers.
- CI runs `pre-commit run --all-files` and then `git diff --exit-code`, so formatting must already
  be applied in the commit. Run `make format` before pushing.
- The hand-written OpenAPI and the served FastAPI schema are compared by `oasdiff` in the
  conformance suite — changing one requires changing the other.
- Deprecate rather than delete: removed fields stay on the published surface marked
  `[deprecated = true]`.

## If you got stuck

A wrong, missing or stale route in this skill is a **bug** — fix it rather than working around it.

- If the file you were sent to did not answer the question, fix the route in this file, and say
  which document *should* have answered it. Reading this from the OSS distribution, you cannot fix
  it there — report it instead, so the fix lands in the source repository and the next bundle.
- If the document exists but was wrong or incomplete, fix the document; this skill only points.
  The usual suspects: `docs/user_manual.md` for semantics, `conformance_tests/README.md` for
  running the suite against a foreign service, `README.md` for build and tooling.
- If you hit a command that does not work, add it to the "Known non-working commands" list above
  with the reason, so the next agent does not spend the same time on it.
- If behaviour was ambiguous in the proto or OpenAPI, the fix belongs in a `.feature` file — that
  is where this project pins semantics the schema cannot express.
