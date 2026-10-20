# Migrate Enterprise Nucleus content to the stack

> **Applies to:** Nucleus Migration Tool (external to this repository) · **Released:** container `migration-tool` 0.1.0 in the Developer Guide 1.3.0 NGC inventory; no chart · **This repo's snapshot:** not present (Nucleus migration was excluded from the import, `IMPORT_STATUS.md`) · **Sources:** the [source skill package](../../NOTICE.md), [published migration guidance](https://docs.omniverse.nvidia.com/ovstorage/ovstorage-guide/latest/dev-journey/migration.html); target-side settings checked against this repo's Storage and Permission Service docs

Nothing about the tool itself can be verified in this repository. Treat CLI subcommands and configuration keys below as the source package's description of the released tool, and confirm them with the acquired image's `generate-config` output. This page is about moving Nucleus content into the stack; for upgrading stack components see [upgrades.md](upgrades.md).

A plan needs: source and target scope, read-only source copies, identity mapping and permission grants, a persistent work directory, content and checkpoint reconciliation, and verification as a restricted user. A migration plan is not migrated data.

## Image coordinate

The release inventory names `nvcr.io/nvidia/omniverse/migration-tool:0.1.0`; the published examples use `nvcr.io/nvidia/omniverse/ov-nucleus-migration:0.1.0`. The source package (2026-10-07) found the `migration-tool` coordinate in the collection and the documented one returning 403. Verify the exact coordinate or digest you are entitled to ([deploy/artifact-access.md](../deploy/artifact-access.md)), inspect its CLI, and set `MIGRATION_IMAGE` explicitly. Inventory fields such as `MIGRATION_IMAGE_REPOSITORY`/`MIGRATION_IMAGE_TAG` in deployment scripts do not launch anything.

## Scope

Migrated, one way, Nucleus → usd-content-workspace: production-branch file content, numbered checkpoints and current heads, translated ACLs, metadata, and optional tags. Not migrated: accounts and passwords, other branches, channels, live objects, mounts, empty folders, deletions. `.thumbs` and `/.system/` are excluded unless enabled.

## Prerequisites

- Nucleus **111 or newer**. Older servers must be upgraded and started once to convert the database before the snapshot.
- Docker with NGC access; copies of `meta.1.1` and `usergroups.1.0`; a read-only mount of `content.1.1`; optional copy of `tags-db/Tagging.db`. Prefer a short maintenance stop for a consistent copy.
- An authenticated target stack with Storage, Permission Service, User Info and Discovery ([deploy/production-stack.md](../deploy/production-stack.md), [deploy/permissions-and-auth.md](../deploy/permissions-and-auth.md), [deploy/userinfo.md](../deploy/userinfo.md), [deploy/discovery.md](../deploy/discovery.md)).
- The destination address is configured in Storage and **empty before the first run**.
- The Permission Service runs in PostgreSQL mode (config-file policies are read-only) and Storage `object` and `folder` resource types use `evaluationPriority: permit`, so translated ACLs keep their decisions ([apis/permissions.md](../apis/permissions.md)).
- The migration identity can write the destination, holds `permissions:edit` and the metadata meta-permission, and can read migrated content and metadata; `permissions:view` for manual inspection. The source package names `permissions:meta`; Permission Service 0.0.52 and later (released 0.0.53) split it into `permissions:meta:read` and `permissions:meta:write` ([CHANGELOG](../../../../permissions/reference/permission-service/CHANGELOG.md)), so grant the new actions on current deployments. Use a dedicated identity and revoke it after cutover.
- A persistent **WORK_DIR** (about 50 GB for large servers, plus database copies), one per source/destination pair, never deleted between runs.

## Configure

```bash
: "${MIGRATION_IMAGE:?set the verified migration image coordinate or digest}"
docker run --rm "$MIGRATION_IMAGE" generate-config > migration_config.toml
```

| Setting | Meaning |
| --- | --- |
| `metadb`, `userdb`, optional `tagsdb` | Snapshot paths inside the container |
| `nucleus_content_dir` or `backup_content_dir` | Live read-only content directory or a backup export; exactly one |
| `progress_dir`, `logs_dir`, `manifest` | Under the persistent work directory |
| `storage_api_url` | The Discovery base URL (used to resolve APIs and login), not the Storage endpoint |
| `destination_address` | Registered destination root under which Nucleus paths are written |
| `aliased_omniverse_url` | Optional original Nucleus URL; configure the matching Storage alias for old checkpoint links |
| `check_permission_evaluation_priority` | Keep enabled |
| `skip_content`, `skip_permissions`, `skip_metadata`, `skip_tags` | Deliberate scope exclusions, not workarounds |
| `include_folder`, `source_path`, `heads_only` | Subsets and history control; `heads_only` loses checkpoint links |

Authentication: `auth_type = "device"` prompts during authenticated phases. Unattended options are `client_credentials`, `refresh_token` and `access_token_file`; keep secrets in the supported secret or token files, not in the TOML or command line. With rotating refresh tokens use `--save-auth`. Token-file mode needs an external renewer.

The dashboard is unauthenticated: listen on `0.0.0.0:10001` inside the container, publish only to host loopback, and reach it through an SSH tunnel.

```bash
run_migration() {
  docker run --rm \
    -v "$SOURCE_DIR":/var/data/source:ro \
    -v "$CONTENT_DIR":/var/data/content:ro \
    -v "$WORK_DIR":/var/data/workdir \
    -p 127.0.0.1:10001:10001 \
    --name nucleus-migration "$MIGRATION_IMAGE" \
    --config /var/data/source/migration_config.toml "$@"
}
```

## Run and verify, one phase at a time

1. `run_migration scan` builds the local manifest; no login, no writes. Review skipped and missing paths. Encoded destination addresses are limited to 1,024 bytes including the prefix; percent-encoding can exceed it.
2. `run_migration retrieve-principals` maps each ACL user's Nucleus email to exactly one User Info identity. Unresolved users lose their permissions; fix real users in `$WORK_DIR/progress/userinfo/principal-overrides.json` and rerun.
3. `run_migration upload` writes content, policies, metadata and tags. Rerun to retry failures; completed work is skipped. Stop gracefully with `docker stop nucleus-migration` and resume with the same work directory.
4. `run_migration scrub` reads everything back without writing. Require zero failures and matching counts; reports are JSONL under `progress/scrub_reports/<timestamp>/`.

Then open migrated content and an old checkpoint link in a client ([components/client-library.md](../components/client-library.md)), and sign in as a user with restricted Nucleus access to confirm visible and denied paths.

## Catch up and cut over

A snapshot misses later Nucleus changes. Take a fresh snapshot, `scan`, rerun `retrieve-principals` if identities changed, `upload` with the same work directory, then `reset-scrub-state` and `scrub` (a new manifest or verification-affecting setting invalidates scrub state; upload progress is kept). For the final cutover make Nucleus read-only or stop it, repeat the cycle, then switch clients. Keep the final snapshot and work directory until no more catch-up is needed.

This is not synchronization: deletions, ACL removals and in-place head edits without a new checkpoint are not propagated, and earlier heads can remain as unreferenced versions.

## Diagnose or reset

| Situation | Action |
| --- | --- |
| Manifest mismatch | Reconcile snapshot and scope, `scan`, reset scrub state |
| Missing principal manifest | `retrieve-principals`; never skip permissions silently |
| Permission-priority rejection | Set object and folder evaluation priority to `permit`; keep the check |
| Many missing blobs | Verify the snapshot and the content mount |
| Alias-only scrub failures | Fix the Storage alias, rerun verification |
| Slow work | Correlate dashboard tasks with Storage and Permission throughput; reduce workers when throttled ([scalability.md](scalability.md)) |
| Target permanently unreachable | Stop gracefully; transient failures retry indefinitely |
| Review policies before upload | `export-policies --output /var/data/workdir/policies.json` (local) |
| Source blob inventory | `export-content-inventory --output /var/data/workdir/inventory.jsonl` |

`reset-upload-state` deletes local upload progress and the manifest, not destination data; rescan, upload and reset scrub state afterwards. `purge-policies` deletes migration-owned policies remotely and is not a retry step. Never hand-edit the work directory.
