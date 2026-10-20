# OSRB

Context for this service's Open Source Review Board submissions. Update this file whenever an
`sapi-osrb` skill operation runs.

## Submissions

| Bug | Scope | Status | Last updated |
|-----|-------|--------|--------------|
| 6482727 | Rust production dependencies of the `microsoft-graph-userinfo-service` container (236 crate versions, 234 components) and base image `distroless/cc`, distributed as a container and Helm chart on NGC | approved | 2026-07-21 |

## Product summary

Microsoft Graph User Info Service is an NVIDIA example service implemented in Rust. It synchronizes users, groups, and
direct memberships from Microsoft Entra ID through Microsoft Graph into PostgreSQL and exposes the resulting directory
through REST and gRPC APIs. It is distributed as a container image and a Helm chart on NGC; third-party Rust source is
shipped in the image at `/app/vendor.tar.bz2`, attributions at `/app/ATTRIBUTIONS.md`, and the PostgreSQL notice at
`/app/ATTRIBUTIONS_POSTGRESQL.md`.

## Shipped packages

The dependency inventory resolves these workspace members, each built into a released container image:

| Member | Evidence |
|--------|----------|
| `microsoft-graph-userinfo-service` | `Dockerfile:27` builds it; `Dockerfile:43` copies the binary into the image |
| `microsoft-graph-userinfo-smoke-test` | `Dockerfile.smoke-test:27` builds it; `Dockerfile.smoke-test:43` copies the binary into the image |
| `userinfo-conformance-test` | `Dockerfile.conformance-test:27` builds it; `Dockerfile.conformance-test:43` copies the binary into the image |

Excluded members:

| Member | Why it never ships |
|--------|--------------------|
| `microsoft-graph-userinfo-publishing` | CI-only tool that packages the API and service documentation into NGC zip archives as part of the release pipeline; no Dockerfile builds it |
| `microsoft-graph-userinfo-licensing` | Local attribution-generation tool; no Dockerfile or CI job builds it |
| `microsoft-graph-userinfo-capabilities-proto` | First-party generated protobuf crate compiled into the service (`crates/microsoft-graph-userinfo-service/Cargo.toml:55`); not resolved separately because its third-party dependencies are part of the service closure |

## Approved exclusions

None. Bug 6482727 records no copyleft or missing-license exclusions; `osrb-copyleft.json` and
`osrb-excluded-licenses.json` do not exist.

## History

- 2026-07-21 — Bug 6482727 approved and closed (submitted 2026-07-20 with `LICENSE-3rdparty.csv`,
  `OSRB_PRODUCTION_DEPENDENCIES.md`, and `OSRB_PRODUCTION_LICENSES.md` attachments).
- 2026-09-23 — Dependency inventory and license attributions regenerated with the bundled `sapi-osrb` scripts
  (`cargo 1.91.1`, `dd-rust-license-tool 1.0.6`, resolved for `x86_64-unknown-linux-gnu`, shipped members only):
  - `osrb-cargo.csv`: 260 components; 30 crates dropped as reachable only from the excluded publishing and
    licensing tools (including the `zip` compression stack); 120 dropped as outside the production set; 4 filled
    from `cargo metadata` (`autocfg`, `cc`, `tonic-prost-build`, `version_check`); no first-party candidates; no
    unresolved `Origin` or `Copyright`.
  - Copyleft gate (`osrb_deps.py check`): no copyleft licenses found.
  - `ATTRIBUTIONS.md`: 260 sections, 0 exclusions; 253 texts from installed packages, 7 from upstream `HEAD`
    (`opentelemetry`, `opentelemetry-otlp`, `opentelemetry-proto`, `opentelemetry_sdk`, `siphasher`, `tonic-prost`,
    `tonic-prost-build`).
  - Changes since the submission (compared by license against the submitted `LICENSE-3rdparty.csv`):
    - Added, runtime dependencies of the service: `adler2` (0BSD OR Apache-2.0 OR MIT), `crc32fast`
      (Apache-2.0 OR MIT), `flate2` (Apache-2.0 OR MIT), `miniz_oxide` (Apache-2.0 OR MIT OR Zlib),
      `simd-adler32` (MIT).
    - Added, build and code-generation dependencies of the shipped members, which the submission did not list:
      `autocfg`, `cc`, `fastrand`, `find-msvc-tools`, `fixedbitset`, `multimap`, `petgraph`, `prettyplease`,
      `regex`, `shlex`, `tempfile`, `unicase`, `version_check`, `windows-link`, `windows-sys` (Apache-2.0 OR MIT);
      `foldhash` (Zlib); `prost-build`, `pulldown-cmark-to-cmark` (Apache-2.0); `pulldown-cmark`, `tonic-build`,
      `tonic-prost-build` (MIT).
    - No longer shipped: none.
    - License differs: none. Eight submitted rows (`fallible-iterator`, `fnv`, `ident_case`, `serde_urlencoded`,
      `siphasher`, `stringprep`, `tagptr`, `unicode-properties`) use the legacy slash notation `MIT/Apache-2.0`,
      which is the same license as the current `Apache-2.0 OR MIT`.
