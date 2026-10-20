# Workspace manifest layer (`.wkspc` / `.manifest`)

How the manifest overlay behaves when you **read** it, and how to **create** one. This is the
practical reference for agents and clients; the full design rationale lives in the
`workspace-manifest-design.md` design doc.

## The model

A folder whose name ends in **`.wkspc`** is a normal, browsable backend folder. A reserved file
inside it named **`.manifest`** optionally defines a **read-only overlay**:

- Paths **listed in the manifest** are served from the resource identity the manifest points at
  (which may live anywhere — another bucket, a shared package store, etc.). These are **read-only**.
- Paths **not** in the manifest **fall through** to the real backend object at that path.
- Writing the `.manifest` file **(re)activates** the overlay.

```
s3://bucket/proj.wkspc/                 ← a normal backend folder
├── .manifest                           ← the overlay definition (a normal object you read/write)
├── top.usd                             ← real backend object (writable, falls through)
└── (overlay-only entries from .manifest, e.g. pkg/sub.usd → served from a shared store)
```

This expresses the wrapp case directly: read-only subpackages are manifest entries pointing at a
shared store; the writable top level is real objects in the `.wkspc` folder.

## Addressing — there is no special endpoint

The overlay triggers purely on the **address**. Use the ordinary storage API (FileObject /
FileFolder / Versioning) with an address under a `.wkspc` segment:

```
<backend-base>/<name>.wkspc/<relative_path>
```

`WorkspaceKey::parse` splits at the **first** `.wkspc` path segment. The remainder is the
relative path within the workspace (percent-decoded and NFC-normalized). A later `.wkspc` segment is
treated as a literal path component (workspaces do not nest in v1).

## Two downstreams: `inner` vs `resolver`

`ManifestLayer` deliberately has two downstream services with different routing contracts:

- **`inner`** handles pass-through delegation and every operation addressed by the client. This
  includes reads of paths not present in the manifest, listing and mutation delegation, and direct
  client access to the `.manifest` object itself.
- **`resolver`** handles only the layer's internal reads: fetching the layer's own `.manifest` body
  and reading a manifest entry by its canonical resource identity.

The split lets permissions sit above the manifest layer while identity resolution can pass through
the remapper and permissions below it. Without that ordering, permission checks cannot govern the
client-facing overlay correctly while canonical identities still resolve through the required
downstream chain.

Preserving overlay semantics imposes a wiring precondition: `.manifest` mutations and direct client
reads and stats go through `inner`, while the layer's internal body read goes through `resolver`
using a path derived from the already-remapped client address. For every `.manifest` address,
`resolver` must reach the same physical object that `inner` reads, writes, or deletes; in particular,
a remapper below `resolver` must be a no-op on post-remap addresses. Otherwise a mutation can
invalidate the manifest cache while subsequent internal reads fetch a different object.

All API-version specializations created with `with_resolver` must share the same
`Arc<LayerCache<CachedManifest>>`. The method preserves that `Arc`; rebuilding a cache per
specialization would allow a `.manifest` write through one API version to leave another version's
cached manifest stale.

`IsVersioned` and `IsReadOnly` answer a workspace path from the loaded manifest. On a cold manifest
cache they read the `.manifest` through `resolver` with the caller's request metadata and
extensions (including `AdditionalHeaders`) plus `CacheControl::NoCache`, so a permission-enforcing
resolver chain authorizes that read as the caller and can fail it. The warm manifest cache is keyed
by workspace path, not by principal: once any caller has loaded a workspace's manifest, later
callers are answered from the cache without a resolver read, so without a permission check on the
`.manifest`.

## Reading

| Operation | In the manifest (`Loaded`) | Not in the manifest | The `.manifest` file itself |
|---|---|---|---|
| `read` / `read_from_address` | read **by identity** from the entry's `resource_identity`; a backend NotFound on that identity propagates | **fall through** to the real backend object | raw bytes of the manifest object |
| `stat` | metadata **from the manifest entry** (no backend read) — always succeeds for an existing entry, never 404s even if the underlying object was deleted | backend `stat` | backend `stat` of the manifest object |
| `list` / `list_stat` (delimited) | merge: backend listing of the folder **+** manifest direct children, manifest wins on a name collision; `.manifest` is **visible** | — | — |
| `enumerate` (recursive) | merge: backend recursive listing **+** manifest entries under the prefix, manifest wins (full-path dedup) | — | — |

Other semantics:
- Returned workspace aliases and manifest-source URLs use storage-core's canonical URL-path
  spelling (RFC 3986 unreserved bytes plus `/`; all other path bytes are percent-encoded). Query and
  fragment components on a source identity are preserved. Cached manifest contents remain raw and
  are canonicalized when rendered, so existing cache entries do not require a schema migration.
- A path that exists in neither the manifest nor the backend → **NotFound** (not a 500).
- Listing order is **backend entries first** (in the backend's native order), then manifest-only
  entries sorted lexicographically. The combined view is intentionally not globally sorted.
- **Writes/deletes/moves of an in-manifest path are rejected** (`MethodNotAllowed`): manifest entries
  are read-only and win. Writes to non-manifest paths go to the backend normally.

## Manifest states

Each workspace resolves to one of three states (keyed by the workspace path):

- **`Loaded`** — `.manifest` parsed and valid → overlay active. **Cached.**
- **`Absent`** — `.manifest` returned NotFound → no overlay; the `.wkspc` folder behaves as a plain
  backend folder (everything falls through). **Not cached** — re-read on every lookup, so a
  `.manifest` written after the workspace was first read/browsed (including by another service
  instance) is picked up immediately instead of staying invisible for the cache TTL. Mutation
  preflights go further: they never *permit* a mutation on a would-delegate result from the coalesced
  cache (`Absent`, or a `Loaded` snapshot missing the target), instead re-reading `.manifest` directly
  and acting on that — so a manifest activated/rewritten concurrently can't be bypassed by a write to a
  now read-only entry. (This closes the stale-cohort window; a manifest activated *after* the
  confirmation but before the backend write is an inherent check-then-act gap, not addressed here.)
- **`Invalid`** — `.manifest` exists but is corrupt/malformed (bad JSON, wrong version, or an invalid
  entry) → **fail-closed**: every overlay-path read/write/list/enumerate is **rejected**, but the
  `.manifest` file itself stays readable/writable/deletable so it can be repaired. **Cached.**

(Transient/permission errors reading `.manifest` propagate as errors and are not cached.)

## Creating a manifest

Three steps. There's no special API — you just write a JSON object to the `.manifest` path.

### 1. Get a resource identity for each target object

For every file you want the overlay to serve, `Stat` the real object through the storage API and take
`resourceInfo.resourceIdentity.encodedIdentity`. That encoded identity (a URL carrying etag /
version-id / etc.) is what the overlay reads by. The target must be a real, readable object.

### 2. Build the `.manifest` JSON

```jsonc
{
  "format_version": { "version_string": "1.0" },   // must be exactly "1.0"
  "storage_api_version": "v1beta",                  // required; "v1alpha" or "v1beta" (not validated)
  "files": [
    // Minimal entry — identity only. stat metadata is derived from the identity URL's query
    // params (size/mtime only if the identity carries them).
    { "relative_path": "pkg/a.usd", "resource_identity": "https://store.example/objects/a?etag=%22abc%22&version-id=..." },

    // Full entry — embeds size + modified_time so stat reports them without parsing the identity.
    // Prefer this for fixtures / when you want stat to return size+mtime.
    {
      "relative_path": "pkg/b.usd",
      "resource_info": {
        "resource_identity": "https://store.example/objects/b?etag=%22def%22",
        "size": 12345,
        "modified_time": "2024-01-01T00:00:00Z"     // RFC 3339
      }
    }
  ]
}
```

Each entry has a `relative_path` and **exactly one** of `resource_identity` (minimal) or
`resource_info` (full) — having both, or neither, is an error.

### 3. Write it to activate

Write the JSON to `<backend-base>/<name>.wkspc/.manifest` via the normal FileObject write
(`write` / `write_by_address`). The manifest layer forwards the write to the backend and **evicts the
cached state**, so the new overlay is active on the next access — even across service instances, and
even for redirect/multipart uploads. Overwriting `.manifest` re-activates with the new contents;
deleting it removes the overlay (the real `.wkspc` objects remain).

### `relative_path` rules (violating any → the whole manifest is `Invalid`, fail-closed)

- must **not** be empty, start with `/`, or contain `.` or `..` segments;
- must **not** contain a `.wkspc` segment (no nested workspaces);
- must **not** be `.manifest` (it can't shadow the real manifest file);
- must be unique (no duplicate `relative_path`);
- write paths as literal decoded UTF-8 (the layer percent-decodes + NFC-normalizes on read).

## Notes

- **Don't cache the manifest object separately:** the layer adds `CacheControl::NoCache` to its
  internal `.manifest` body read through `resolver` and to pass-through client `read_from_address` /
  `stat` requests for `.manifest` through `inner`, so stat/object caches below either downstream
  cannot retain a stale copy.
- **Test fixtures:** to seed `.wkspc` fixtures + generate their `.manifest` files on the CICD
  backends, use the `bump-manifests` skill (`.claude/skills/bump-manifests/`) + the spec at
  `tools/manifests/workspace-fixtures/manifest-spec.json`. Don't hand-edit baked identities — they
  go stale when the underlying object's identity format changes.
- Constants: the suffix `.wkspc` and filename `.manifest` are defined once in
  `format.rs` (`WORKSPACE_SUFFIX`, `MANIFEST_FILENAME`).
