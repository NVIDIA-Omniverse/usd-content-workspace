(thumbnail-service-destination-path-template-thumbnail-destination-path-template)=
# Thumbnail destination path template

Thumbnail Service decides **where every generated thumbnail is written**. It resolves one exact, absolute storage address per thumbnail from a single operator-supplied template, `THUMBNAIL_PATH_TEMPLATE` (Helm value `thumbnailPathTemplate`), and sends that address as `destination_resource_address` on every `omni.thumbnail.generate` event. The generation worker writes the blob to exactly that address — it holds no template and performs no derivation of its own. Event-driven generation, `POST /v1alpha/thumbnails`, and gRPC `CreateThumbnails` all go through the same resolution, so the layout of the thumbnail tree is operator policy expressed once, not code.

The template is **required**: there is no default, and the service fails fast at startup when the value is missing or cannot satisfy the rules below. The variables it may reference are fixed and substituted verbatim; there are no functions, conditionals, or per-request overrides.

For how the resolved destination travels to the worker, see {ref}`Thumbnail generation worker <thumbnail-service-generation-pipeline-thumbnail-generation-worker>`. For the Helm value that carries the template, see {ref}`Thumbnail Service Configuration <thumbnail-service-deployment-thumbnail-service-configuration>` and {ref}`Environment configuration reference <thumbnail-service-configuration-environment-configuration-reference>`.

(thumbnail-service-destination-path-template-supported-variables)=
## Supported variables

| Variable | Meaning | Example |
|---|---|---|
| `{resource_path}` | Directory portion of the source `resource_address`, including scheme and host, with **no** trailing slash. At a container root it expands to the container URL without a trailing slash. | `https://acct.blob.core.windows.net/assets/photos` |
| `{resource_filename}` | Filename portion of the source address, with extension. | `image.jpg` |
| `{resource_address_md5}` | Lowercase hex MD5 of the complete source `resource_address`. | `475673a39cfd8a2fb955ee5c352c1082` |
| `{resource_identity_md5}` | Lowercase hex MD5 of the complete source `resource_identity`. | `fc0bb2a4bbc3bbb3ff568cde8275a2bf` |
| `{width}` | Requested thumbnail width in pixels. | `256` |
| `{height}` | Requested thumbnail height in pixels. | `256` |
| `{output_encoding}` | Requested encoding format, **lowercased**. | `png` |
| `{quality}` | Resolved encode quality (the caller's value or `THUMBNAIL_QUALITY`). | `85` |

Substitution is plain text replacement: each `{var}` is replaced by its value and nothing else. The MD5 variables hash the complete string of the value they name.

(thumbnail-service-destination-path-template-validation-at-startup)=
## Validation at startup

The service parses the template before binding a listener and refuses to start when any of the following holds:

| Check | Startup error |
|---|---|
| Malformed braces or an unknown `{var}` reference | Names the offending variable; a typo never silently substitutes as an empty string. |
| No source-identifying variable | `thumbnail path template must identify its source with '{resource_address_md5}' or both '{resource_path}' and '{resource_filename}'` — without one of these, every source of a given size renders to the same path and overwrites the others' thumbnails. |
| Rendered address is not absolute | `thumbnail path template must render an absolute storage address` — a smoke render must produce a URL with a scheme and host. |
| Rendered shape cannot be recognized again | Two greedy variables adjacent with no literal between them (for example `{resource_path}{resource_filename}` or `{width}{height}`) are rejected, because the {ref}`recursion guard <thumbnail-service-destination-path-template-generated-output-recursion-prevention>` could not match the rendered output. |

Helm does **not** validate template semantics — `thumbnailPathTemplate` is a plain required string at render time, so a template that passes `helm template` can still stop the pod at startup. At render time each source address must parse as a URL with a host; when `{resource_path}` or `{resource_filename}` is used, the source must also have a non-empty final path segment. The rendered destination is re-parsed and re-serialized into canonical URL form.

(thumbnail-service-destination-path-template-examples)=
## Examples

**Centralized, content-hash-keyed.** Every thumbnail lands under one shared `thumbnails/` tree, keyed by a hash of the source address plus a hash of its identity, so collisions across sources are avoided without touching the source container:

```text
https://acct.blob.core.windows.net/acct/thumbnails/{resource_address_md5}/{resource_identity_md5}/{width}x{height}/q{quality}/{resource_filename}.auto.{output_encoding}
```

**Centralized, flat.** The same idea, one level simpler — `{resource_address_md5}` alone still uniquely identifies the source:

```text
https://acct.blob.core.windows.net/acct/thumbnails/{resource_address_md5}/{width}x{height}/{resource_filename}.auto.{output_encoding}
```

**Source-relative.** A `.thumbs` directory next to each source file, identifying the source with `{resource_path}` and `{resource_filename}` together instead of an address hash. For the source `https://foo.example/path/file.jpg` and a 256×256 PNG specification:

```text
{resource_path}/.thumbs/{width}x{height}/{resource_filename}.auto.{output_encoding}
```

renders

```text
https://foo.example/path/.thumbs/256x256/file.jpg.auto.png
```

**Invalid — no source identifier.** Rejected at startup: every source of a given size would render to the same path.

```text
https://acct.blob.core.windows.net/acct/{width}x{height}/thumbnail.{output_encoding}
```

**Invalid — not absolute.** Rejected at startup: a bare container-relative path does not resolve to a storage address.

```text
acct/{resource_address_md5}/{width}x{height}.{output_encoding}
```

(thumbnail-service-destination-path-template-verbatim-substitution)=
## Verbatim substitution

No variable contributes a separator of its own, so every slash in the rendered address is one the template wrote:

| Template fragment | Renders for source `https://foo.example/path/file.jpg` | Effect |
|---|---|---|
| `{resource_path}/.thumbs/…` | `https://foo.example/path/.thumbs/…` | A `.thumbs` directory **under** the source directory. |
| `{resource_path}//.thumbs/…` | `https://foo.example/path//.thumbs/…` | Two separators — a deliberate empty path segment. |
| `{resource_path}.thumbs/…` | `https://foo.example/path.thumbs/…` | A **sibling** directory whose name ends in `.thumbs`. |

A source at the container root, `https://foo.example/file.jpg`, renders `{resource_path}/.thumbs/…` as `https://foo.example/.thumbs/…`. Every separator form is recognized by the recursion guard described below.

(thumbnail-service-destination-path-template-deterministic-resolution-and-versions)=
## Deterministic resolution and versions

Resolution is a pure function of the source address, the source identity, the specification, and the template: identical inputs produce an identical destination on the event-driven and API paths alike. This is what makes the worker idempotent on redelivery — a duplicate `omni.thumbnail.generate` overwrites the same destination rather than creating a second blob — and what makes a re-association converge on the same metadata key (see {ref}`Deterministic key <thumbnail-service-associations-deterministic-key>`).

Whether a **new version** of a source produces a new thumbnail file is a template decision:

- Without `{resource_identity_md5}`, a new `resource_identity` of the same address resolves to the **same** destination, and the previous thumbnail is overwritten.
- With `{resource_identity_md5}`, a new identity resolves to a **different** destination, and thumbnails of earlier versions remain in storage.

Changing the template (or the configured `THUMBNAIL_SIZES`) after thumbnails exist makes previously generated blobs unreachable by derivation. They are not moved or deleted automatically; the storage-deleted sweep still reclaims them because it enumerates association metadata rather than re-rendering the template (see {ref}`Blob deletion policy <thumbnail-service-associations-blob-deletion-policy>`).

(thumbnail-service-destination-path-template-generated-output-recursion-prevention)=
## Generated-output recursion prevention

A thumbnail write is itself a storage CREATE event under a watched prefix. Without a guard the service would re-submit its own output for generation, appending another `.auto.<ext>` on every pass. Two checks run, in order, on every `omni.storage.created` delivery, independent of which layout the template configures:

1. **Event markers and writer principal.** An event carrying `generated_by_thumbnail_pipeline: true` is skipped. When `GENERATED_THUMBNAIL_PRINCIPAL_IDENTITY` is configured, an event whose `principal_identity` equals it is skipped as well; the value names the principal under which the generation worker writes. Storage does not yet stamp either signal on CREATE events, so this check is **inert in every environment today**. The variable has no Helm value; inject it through `thumbnailService.extraVars` if an environment needs it.
2. **Template-output shape matching.** An address whose shape matches the configured template's rendered output is skipped. The match is structural: literals must match exactly, `{resource_address_md5}` and `{resource_identity_md5}` slots accept 32 hex characters, `{width}`, `{height}`, and `{quality}` accept digits, `{output_encoding}` accepts the supported encodings, and `{resource_path}` / `{resource_filename}` match greedily up to the next literal. Query strings and fragments are stripped before matching. This is the guard that carries recursion prevention in practice.

The check is stateless — a pure function of the configured template and the event's address — so it does not depend on which replica handles an event, and the orchestrator's replica count is not constrained by it.

```{note}
The shape match has one documented limitation: a legitimately uploaded source whose address happens to match the template's rendered shape is skipped as if it were generated output. Choose a template whose literals (for example a dedicated `thumbnails/` tree or a `.auto.` marker) do not occur in ordinary source addresses.
```

Skipped events are acknowledged and logged at `debug` level with the reason (`Skipping CREATE event`); see {ref}`Eligibility <thumbnail-service-event-consumption-eligibility>` for the complete ordered list of checks.

(thumbnail-service-destination-path-template-see-also)=
## See also

- {ref}`Storage event consumption <thumbnail-service-event-consumption-storage-event-consumption>` — where the recursion guard runs and how eligible events fan out into generate work.
- {ref}`Thumbnail generation worker <thumbnail-service-generation-pipeline-thumbnail-generation-worker>` — the destination-agnostic writer that uploads to the resolved address.
- {ref}`Association metadata and queries <thumbnail-service-associations-association-metadata-and-queries>` — how the destination becomes the associated thumbnail's `resource_address`, and how the sweep reclaims generated blobs.
- {ref}`Environment configuration reference <thumbnail-service-configuration-environment-configuration-reference>` — `THUMBNAIL_PATH_TEMPLATE`, `THUMBNAIL_SIZES`, and the `THUMBNAIL_*` defaults that feed `{width}`, `{height}`, `{output_encoding}`, and `{quality}`.
- {ref}`Thumbnail Service Configuration <thumbnail-service-deployment-thumbnail-service-configuration>` — setting `thumbnailPathTemplate` in the Helm values file.
