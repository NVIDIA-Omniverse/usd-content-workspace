# Workspace-manifest integration test fixtures

Static fixtures for the `.wkspc`/`.manifest` overlay integration tests
(`storage-core-integration-tests/tests/manifest.rs`). See
`docs/workspace-manifest-integration-test-plan.md` for the full plan.

These mirror the layout to seed under `unittests/workspaces/` on **all six** read buckets
(S3 + Azure × public/private/versioned). The old `unittests/manifests/` `.omnimfst` fixtures are
left untouched for trunk/other branches.

## What's here (static — uploaded as-is)

- `sources/**` — the real objects the overlay manifests point at (stable across versions).
  - `sources/blob.bin` is a small (1 KiB) committed binary used as the redirect-read target (the
    `folder_b/jellyfish - hevc.mkv` manifest entry points at it). Redirect works at any size.
- `overlay_v1.wkspc/**` (except `.manifest`) — real backend objects that merge with / are shadowed
  by the overlay, plus fall-through objects.
- `absent_v1.wkspc/**` — a workspace with no `.manifest` (plain folder).
- `invalid_v1.wkspc/.manifest` — intentionally malformed JSON (fail-closed test); `real_file.txt` is
  a normal object.

File contents are tiny, fixed strings (e.g. `shadowed.txt` = `FROM-BACKEND` vs the manifest source
`shadow_source.txt` = `FROM-MANIFEST`, to prove manifest-wins on read).

## What's generated (NOT here)

`overlay_v1.wkspc/.manifest` and `manifest_only_v1.wkspc/.manifest` bake per-backend resource
identities (incl. `version-id` on versioned buckets), so the `bump-manifests` skill generates them
by `Stat`-ing the `sources/*` objects. The mapping (relative_path → source object) is in
`manifest-spec.json`.

## Encoding note

`my docs/` (space) is the deterministic encoding-dedup case: backends return `my%20docs/`, the
manifest key is the literal space, and the layer decodes before the shadow-set compare. The
NFC-mismatch (decomposed vs precomposed) case is unit-tested instead (backends may normalize names).
`café.txt` here uses precomposed `é` (U+00E9) in both the manifest entry and the backend object.

## Versioning

`sources/` is shared/unversioned. Workspace folders carry a `_vN` suffix. On an identity-format
change, bump to `_v2` (re-seed `_v2.wkspc` objects + regenerate `.manifest`s), update the test's
workspace-folder constant, and leave `_v1` for other branches.
