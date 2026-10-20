---
name: storage-navigator-cli
description: Operates the Storage Navigator CLI (`sn`) to inspect and manage Omniverse Storage resources with agent-friendly output. Use when an agent needs to browse, transfer, tag, archive, version, or change storage resources through `sn`, or interpret its JSON results and errors.
---

# Storage Navigator CLI

## Quick start

1. Confirm `sn` is installed (`sn --help`) and inspect configured hosts with `sn --json host list`.
2. Use `sn --json --host <name>` for finite commands. Select a host per invocation; avoid changing the shared active host with `host use`.
3. Start with `sn --json --host <name> ls <folder/>`. Reuse the returned `resourceAddress` values for later commands.
4. Check the exit status. Success data is on stdout; a nonzero exit writes one JSON error object to stderr.

```bash
sn --json host list
sn --json --host dev ls 'https://bucket.example.com/bucket/Library/'
sn --json --host dev meta 'https://bucket.example.com/bucket/Library/scene.usd'
```

If no host is configured or authentication is required, ask the user to run `sn host add <name> <discovery-url>` interactively. `--json` disables interactive device login. Do not display or edit saved tokens in `~/.config/sn/config.json`.

## Choose a path

- Prefer full resource addresses from `ls --json`. Relative paths depend on a working folder shared by processes.
- Include a trailing `/` when a path denotes a folder, especially for `get`, `meta`, `tags`, and `archive`.
- Quote addresses and local paths in shell commands. The CLI does not implement remote glob patterns.

## Inspect before changing storage

| Task | Command after `sn --json --host <name>` |
| --- | --- |
| Browse | `ls <folder/>` or `ls -l <folder/>` |
| Inspect metadata and ACL | `meta <file-or-folder>` |
| Read tags | `tags list <file-or-folder>` |
| Inspect versions | `versions list <file>` |
| Check advertised services | `host services <name>` |

For `put`, `cp`, `mv`, and `rm`, run `--dry-run` first when changing multiple resources or recursively traversing folders. Review the resolved source and destination addresses. `put`, `cp`, and `mv` default to `--on-conflict fail`; choose `skip` or `overwrite` only when the task calls for it.

```bash
sn --json --host dev cp --dry-run '<source/>' '<destination/>'
sn --json --host dev cp --on-conflict fail '<source/>' '<destination/>'
sn --json --host dev rm -r --dry-run '<folder/>'
```

## Common actions

| Task | Command after `sn --json --host <name>` |
| --- | --- |
| Download file or folder | `get <remote> [local-dest]` |
| Upload file or folder | `put <local> [remote]` |
| Create folder | `mkdir <folder>` |
| Copy or move | `cp <src> <dst>` / `mv <src> <dst>` |
| Delete file or folder | `rm <file>` / `rm -r <folder/>` |
| Set or remove tag | `tags set <path> <key> <value>` / `tags remove <path> <key>` |
| Restore version | `versions restore <file> <resourceIdentity>` |
| Download server ZIP | `archive <paths...> --output <local.zip>` |
| Watch events | `notification --path <folder/> <types...>` |

Tag edits read the current ETag and use it for the update. On `CONFLICT`, reread tags before retrying. `archive` needs an advertised archive service and preserves an existing local file unless `--overwrite` is set. `get` likewise needs `--overwrite` to replace a local file.

## Interpret results

- `ls` and `versions list` return JSON arrays; `meta` and `tags list` return JSON objects.
- Successful mutations return one object with `ok: true` and a `command` field.
- Errors return `ok: false` with `error.code` and `error.message` on stderr. A `PARTIAL_FAILURE` can mean some batch items already completed; inspect `error.failures` before retrying.
- `notification --json` emits one JSON object per line until stopped. Treat it as a stream rather than one finite JSON value.
- When piping JSON through `jq` or into another `sn` command, enable `set -o pipefail` and inspect the pipeline's exit status.
- Permission management has no CLI commands yet. `meta` reports ACL metadata but does not change grants.

For exact syntax and examples, see the repository [README](../../../README.md).
