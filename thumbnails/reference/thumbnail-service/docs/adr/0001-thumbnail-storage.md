<!-- Amendment (rust-thumbnail-task-worker): every reference below to
     `thumbnail-task-queue` as the writer now means `crates/task-worker`. The
     decision is unchanged — the writer holds no template and performs no
     derivation, and receives one fully-resolved destination per work item — and
     the text is left as written rather than rewritten, because an ADR records
     what was decided when. What changed is the transport: the resolved
     destination travels on an `omni.thumbnail.generate` event instead of an
     HTTP request body. -->

# Thumbnail storage: pure derivation, template-driven path

## Status

accepted

## Context

Thumbnails are generated from source resources (images in remote storage) and written back to storage. The original layout used a flat directory with UUID-stamped filenames (`{thumbnail_storage_path}/thumbnail_{w}x{h}_{fmt}_{uuid}`), which made it impossible to find a source's thumbnails without consulting this service's association records. That blocked any layout matching what downstream readers expect, and forced all lookup paths through the association table.

## Decision

Thumbnail storage location is a **pure function of `(source_resource_address, source_resource_identity, specification)`** — no database or association lookup is required to compute where a thumbnail lives.

The path is produced by substituting variables into a single operator-supplied template, `thumbnail_path_template`, configured on **this service**. thumbnail-task-queue holds no template and performs no derivation; it accepts a fully-resolved destination address per generate request.

### Template variables

Plain `{var}` substitution only. No function calls in the template DSL. The ADR may be updated if other spec fields are needed.

| Variable                   | Source                                          | Example                              |
| -------------------------- | ----------------------------------------------- | ------------------------------------ |
| `{resource_path}`          | Directory portion of source address (with scheme and host, **no** trailing slash) | `https://foo.com/path`               |
| `{resource_filename}`      | Filename portion of source address (with extension) | `file.jpg`                           |
| `{resource_address_md5}`   | MD5 of the full source resource address         | `8b6066dedf41dc0ea3a29e45192f9727`   |
| `{resource_identity_md5}`  | MD5 of the full source resource identity        | `fc0bb2a4bbc3bbb3ff568cde8275a2bf`   |
| `{width}`                  | Specification width                             | `256`                                |
| `{height}`                 | Specification height                            | `256`                                |
| `{output_encoding}`        | Specification encoding format, **lowercased**   | `png`                                |
| `{quality}`                | Specification output quality                    | `85`                                 |

### Examples

- source resource address: `https://foo.com/path/file.jpg`
- source resource identity: `https://foo.com/path/file.jpg?versionid=12345`
- specification: `256x256 PNG`

#### Static thumbnail folder

thumbnail path template: `https://foo.com/_thumbnails/{resource_address_md5}/{resource_identity_md5}-{resource_filename}.{output_encoding}`

Resulting thumbnail resource address:

```
https://foo.com/_thumbnails/8b6066dedf41dc0ea3a29e45192f9727/fc0bb2a4bbc3bbb3ff568cde8275a2bf-file.jpg.png
```

#### Colocate thumbs with source (Legacy Nucleus thumbnail support)

- `thumbnail_path_template`: `{resource_path}/.thumbs/{width}x{height}/{resource_filename}.auto.{output_encoding}`

Resulting thumbnail resource address:

```
https://foo.com/path/.thumbs/256x256/file.jpg.auto.png
```


### Validation

The template is required config — there is no default. At startup this service rejects:

- Unknown `{var}` references (typos must not silently substitute as empty).
- Templates with no source-identifying variable. The template must reference **either** `{resource_address_md5}` **or both** `{resource_path}` and `{resource_filename}`. Without one of these, distinct sources collide at the same path, causing silent data loss.

### Unique thumbnails per version

- If unique thumbnail files are desired the template string must include `{resource_identity_md5}`. Omitting `{resource_identity_md5}` will result in a single thumbnail that is overwritten on each new version of the resource. 

### Generate flow

For each generate request this service handles:

1. Derive the destination address from `(source_resource_address, source_resource_identity, specification)` via `thumbnail_path_template`.
2. Call thumbnail-task-queue with the **fully-resolved** destination address as a required field on the request.
3. thumbnail-task-queue writes the blob to exactly that address.
4. This service records the (source ↔ thumbnail) association on success.

Derivation is unconditional: every generate request (event-driven, HTTP, gRPC) resolves its destination from `thumbnail_path_template`. There is no per-request override — see "Considered alternatives" below.

Generated-output recursion is prevented independently of destination layout, via two checks in order of preference:

1. Storage events carrying `generated_by_thumbnail_pipeline: true` or, when configured, the matching writer `principal_identity` are honored. Neither is populated by storage today (events carry the storage notification client's principal, not the writer's, and nothing sets the marker); this remains the intended long-term signal once storage propagates one.
2. Otherwise, the service skips addresses whose shape matches the configured `thumbnail_path_template` output. A normal source under a path that also contains generated thumbnails remains eligible when its address does not match that rendered shape.

## Consequences

- **thumbnail-task-queue is a dumb blob writer with respect to path.** It holds no template, performs no derivation. Its generate API requires a fully-resolved destination address. Any caller that wants a thumbnail at a specific path must compute it (or use this service, which does).
- **Single source of truth for path layout.** `thumbnail_path_template` lives only on this service. There is no second copy in thumbnail-task-queue to drift.
- **Layout is operator policy, not code.** The shape of the path (hash-based, colocated, base-prefixed, etc.) is fully expressed in `thumbnail_path_template`. The service has no opinion. Legacy-consumer expectations (e.g. the `.thumbs/{w}x{h}/{filename}.{encoding}` layout one downstream reader requires) are satisfied by configuring the template appropriately; they are no longer baked into code.
- **Hard break on previous config keys.** `storage_strategy`, `thumbnail_base_address`, and `thumbnail_folder_name` are removed. The new `thumbnail_path_template` is required — this service fails to start without it. Every deployment must be touched at rollout. No translation shim.
- **Loop prevention uses event markers, an optional writer principal, and template-output shape.** Before enabling event-driven generation in an environment, deployments must verify that a thumbnail written by `task-worker` — the writer of generated output after the Rust cutover — does not enqueue another generation task. When storage propagates writer principal or `generated_by_thumbnail_pipeline`, configure `GENERATED_THUMBNAIL_PRINCIPAL_IDENTITY` to match the writer and rely on those markers. Until then, the service skips CREATE events whose address matches the configured template's rendered output shape. Re-enable event consumption only after this verification passes in the target environment.
- **Existing UUID-stamped thumbnails are abandoned** at cutover. They will be cleaned up manually outside this codebase. Sources will regenerate thumbnails under the new scheme on next request.
- **Config-change orphans are accepted.** If `thumbnail_path_template` or the canonical `w×h` changes, thumbnails generated before the change become unreachable by derivation. Mitigation for live-source config-change orphans remains operational (one-time migration), not in-code.

## Considered alternatives

- **Two first-class strategies (`resource` and `colocate`) selected by an enum.** An earlier revision of this ADR. Rejected: doubled config surface, embedded layout in code, and still couldn't express variants close to but not exactly matching either strategy without growing the enum.
- **Function calls in the template DSL** (e.g. `{md5(resource_address)}`). Rejected: introduces a mini-language to parse, validate, and maintain. Pre-computed variables (`{resource_address_md5}`) cover the only function anyone wanted.
- **Default template baked in.** Rejected: a wrong default silently writes thumbnails to a path the operator didn't expect. Forcing explicit config catches this at startup, not in production.
- **Per-request `destination_resource_address` override.** Initially implemented, exposed additively on the HTTP and gRPC generate APIs, bypassing template derivation when supplied. Removed before this feature shipped: no confirmed consumer demand, and not part of the SRD/PRD. It was also the sole reason for an in-process, TTL-bounded "recent destination" cache in the loop-prevention design (an override by definition need not match the template's rendered shape, so shape-matching alone couldn't catch it) — removing the override let that cache be removed too, simplifying loop prevention back to two checks. Can be reintroduced, scoped to actual requirements, if requested.
- **Read-through fallback to the old UUID-stamped layout.** Rejected: defeats pure derivation, complicates reads permanently for a one-time migration concern.
