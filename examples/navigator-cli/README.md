# sn — Storage Navigator CLI

A command-line tool for browsing and managing files in Omniverse Storage.

## Installation

Requires Node.js 22.12+ on the 22.x release line, Node.js 24.x, or Node.js 26+, and npm. Clone this repository, open a terminal in its root, then run:

```bash
npm ci
npm run build
npm link
sn --help
```

This builds the CLI and links the `sn` binary globally so it's available on your PATH. To uninstall:

```bash
npm unlink -g @storage-navigator/cli
```

To build an installable archive from source, run `npm pack`. Packing rebuilds the CLI and includes only the built files, README, license, and CLI operating skill.

## Getting Started

### Add a host

```bash
sn host add <name> <discovery-url>
```

Authenticates via device flow and saves the host to `~/.config/sn/config.json`.
Run this setup interactively before using the CLI from an unattended agent. Commands run with `--json` or `--non-interactive` will not start a device login.

```bash
sn host add dev https://discovery.example.com
```

### Switch hosts

```bash
sn host use <name>
sn host list
sn host remove <name>
sn host services [name]    # list services available on a host
```

## Using `sn` from an agent

Use `--json` to get one JSON value from a finite command. Put `--host <name>` on each command when multiple agents or hosts share a machine, and use the `resourceAddress` returned by `ls` for later operations. Full resource addresses avoid the shared working folder setting.

```bash
sn --json host list
sn --json --host dev ls mybucket/Library/
sn --json --host dev meta 'https://mybucket.blob.core.windows.net/mybucket/Library/scene.usd'
sn --json --host dev cp --dry-run 'https://mybucket.blob.core.windows.net/mybucket/Library/scene.usd' 'https://mybucket.blob.core.windows.net/mybucket/Archive/scene.usd'
sn --json --host dev cp --on-conflict fail 'https://mybucket.blob.core.windows.net/mybucket/Library/scene.usd' 'https://mybucket.blob.core.windows.net/mybucket/Archive/scene.usd'
```

Read command shapes differ: `ls` and `versions list` return arrays; `meta` and `tags list` return objects. A successful mutation returns `{ "ok": true, "command": "...", ... }`. Failures exit nonzero and write one JSON error object to **stderr**, leaving stdout empty. Check the exit status before using output. `notification --json` streams newline-delimited JSON events until stopped.

`put`, `cp`, and `mv` default to `--on-conflict fail`. For recursive or bulk changes, inspect `--dry-run` output first and choose `fail`, `skip`, or `overwrite` deliberately. Batch commands can partially complete; a `PARTIAL_FAILURE` error includes failed items and completed work. Recursive mutations also report `completedOperations` across earlier folders. On a worker failure, queued work stops and active transfers finish before the command exits. Stdin batches collect per-line failures and continue to the next line.

Agents working in this repository can read the [Storage Navigator CLI skill](.agents/skills/storage-navigator-cli/SKILL.md) for a short operating workflow.

## Path Formats

All file commands accept paths in any of these forms. Remote paths are URI paths: encode literal `#`, `?`, and `%` characters as `%23`, `%3F`, and `%25`. Uploads automatically encode names derived from the local filesystem, including spaces and literal percent signs.

| Format              | Example                                                    | Resolves to                                  |
| ------------------- | ---------------------------------------------------------- | -------------------------------------------- |
| Full HTTPS URL      | `https://mybucket.blob.core.windows.net/mybucket/file.usd` | used as-is                                   |
| Bucket-relative     | `mybucket/Library/scene.usd`                               | looked up by bucket name                     |
| Bare bucket         | `mybucket`                                                 | top-level bucket address                     |
| Relative (explicit) | `./scene.usd`                                              | prepended with current working folder        |
| Bare name           | `scene.usd`                                                | prepended with current working folder if set |

Folder paths should end in `/` for `get`, `meta`, `tags`, and `archive` when the intended resource is a folder. A full resource address is the most predictable form for automation.

## Current Working Folder

Set a working folder to avoid typing full paths on every command:

```bash
sn cd mybucket/Library
# Working folder is saved as the resolved full address.

sn ls                    # lists mybucket/Library/
sn meta scene.usd        # shows metadata for mybucket/Library/scene.usd
```

To reset to the root (clear the working folder), run `sn cd` with no argument or with `/`:

```bash
sn cd       # clears working folder
sn cd /     # same effect
```

The working folder is persisted per host in config.

## Commands

### `sn pwd`

Print the current working folder for the active host.

```bash
sn pwd
# https://mybucket.blob.core.windows.net/mybucket/Library/
```

### `sn ls [path]`

List folder contents. With no path and no working folder set, lists top-level buckets. Both short and long JSON listings use `name`, `type` (`file` or `folder`), and `resourceAddress`; folder names omit the display-only trailing slash. Long listings also include available size and modification time.

```bash
sn ls
sn ls mybucket/Library
sn ls -l mybucket/Library    # long format with size and date
sn ls --json mybucket        # JSON output
sn --host dev ls --json mybucket  # select a host for this invocation
```

### `sn meta <path>`

Show file or folder metadata, including ACL and tags (with their ETags).

```bash
sn meta mybucket/Library/scene.usd
sn meta --json mybucket/Library/
```

### `sn tags list|set|remove`

Read or edit file and folder tags. Edits read the current tag ETag and use it for the update, so a concurrent change returns a conflict instead of silently replacing it.

```bash
sn tags list mybucket/Library/scene.usd
sn tags set mybucket/Library/scene.usd project demo
sn tags remove mybucket/Library/scene.usd project
```

### `sn archive <paths...> --output <file.zip>`

Download a server-generated ZIP when the host advertises an archive service. Include a trailing slash for folder paths. An existing local file is preserved unless `--overwrite` is passed.

```bash
sn archive mybucket/Library/scene.usd mybucket/Textures/ --output bundle.zip
```

### `sn get [remote-path] [local-dest]`

Download a file or folder. Defaults to the current local directory if no destination is given. Errors if the local file already exists unless `--overwrite` is passed.

For folders, include a trailing slash — `sn get mybucket/Library/` downloads all files recursively into a local `Library/` directory. The folder name is always appended to the local destination.

Downloads preserve empty directories and validate remote names and listings before writing. Folder downloads reject symlink directories beneath the selected local destination. Each file downloads to a temporary file before an atomic commit: existing files are preserved on transfer failure, and concurrent downloads cannot replace a file without `--overwrite`. Piped file inputs with duplicate local filenames report a batch conflict.

```bash
sn get mybucket/Library/scene.usd
sn get mybucket/Library/scene.usd ~/Downloads/scene.usd
sn get mybucket/Library/                        # download folder → ./Library/
sn get mybucket/Library/ ~/dest/                # download folder → ~/dest/Library/
sn get --progress mybucket/Library/scene.usd    # show transfer progress
sn get --progress mybucket/Library/             # show per-file progress for folder
sn get --overwrite mybucket/Library/scene.usd   # replace existing local file
sn get --overwrite mybucket/Library/            # replace existing local files in folder
```

**Stdin piping:** If `remote-path` is omitted and stdin is piped, each line is treated as a remote path and downloaded into the current local directory. Lines ending with `/` are treated as folders. A positional argument is interpreted as `remote-path`, so the stdin form cannot specify `local-dest`.

```bash
sn --json ls mybucket/Library/ | jq -r '.[] | select(.type == "file") | .resourceAddress' | sn get
```

Use `set -o pipefail` when scripting this pipeline so a failed `ls` or `jq` does not look like a successful empty download.

### `sn put [local-path] [remote-path]`

Upload a file or folder. If no remote path is given, uploads to the current working folder using the local filename. Requires a working folder to be set if remote path is omitted. For folders, the local folder name is appended beneath the remote destination.

Folder uploads preserve empty directories. Dry runs include the folders that will be created.

```bash
sn put ~/scene.usd mybucket/Library/scene.usd
sn put ~/scene.usd                              # uploads to <cwd>/scene.usd
sn put --progress ~/scene.usd mybucket/Library/scene.usd   # show transfer progress
sn put --dry-run ~/scene.usd mybucket/Library/scene.usd    # preview uploads
sn put --on-conflict skip ~/scene.usd mybucket/Library/scene.usd
```

**Stdin piping:** If `local-path` is omitted and stdin is piped, each line is treated as a local path and uploaded to `cwd/<basename>`. `remote-path` cannot be used in stdin mode (each file must land in the current folder).

```bash
find . -name "*.usd" | sn put   # upload all USD files to cwd
```

### `sn mkdir <path>`

Create a folder.

```bash
sn mkdir mybucket/Library/NewFolder
```

### `sn rm [path]`

Delete a file or folder. Use `-r` for folders. Preview recursive deletions with `--dry-run`.

```bash
sn rm mybucket/Library/scene.usd
sn rm -r mybucket/Library/OldFolder
sn rm -r --dry-run mybucket/Library/OldFolder
```

**Stdin piping:** If `path` is omitted and stdin is piped, each line is deleted. The `-r` flag applies to every line.

```bash
sn --json ls mybucket/Library/ | jq -r '.[] | select(.type == "file" and (.name | startswith("tmp"))) | .resourceAddress' | sn rm --dry-run
```

Use `set -o pipefail` in a script, inspect the preview, then rerun the same selection without `--dry-run` if deletion is intended.

### `sn mv <src> <dst>`

Move or rename a file or folder. Include trailing `/` on folder paths.

Folder moves create destination folders, including empty directories, before removing their sources. A failed destination creation leaves the corresponding source folder intact.

```bash
sn mv mybucket/Library/old.usd mybucket/Library/new.usd
sn mv --dry-run mybucket/Library/old.usd mybucket/Library/new.usd
```

### `sn cp <src> <dst>`

Copy a file or folder. Include trailing `/` on folder paths.

Folder copies preserve empty directories. Copy and move dry runs include destination folder creation.

```bash
sn cp mybucket/Library/scene.usd mybucket/Archive/scene.usd
sn cp --on-conflict overwrite mybucket/Library/scene.usd mybucket/Archive/scene.usd
```

### `sn versions list <path>`

List available versions of a file.

```bash
sn versions list mybucket/Library/scene.usd
sn versions list --json mybucket/Library/scene.usd
```

### `sn versions restore <path> <identity>`

Restore a previous version of a file in-place. Use the identity from `versions list`.

```bash
sn versions restore mybucket/Library/scene.usd <identity>
```

### `sn notification <types...>`

Watch for notification events in the current working folder. Requires a working folder to be set with `sn cd`. One or more event types must be specified. Press Ctrl-C to stop.

```bash
sn notification omni.storage.created
sn notification omni.storage.created omni.storage.deleted omni.thumbnail.associated
sn notification --json omni.storage.created    # one JSON object per event
sn notification --path mybucket/Library/ --json omni.storage.created
```

Events are printed to stdout as they arrive. The status line is written to stderr so it can be suppressed independently if needed.
With `--json`, each event is one compact JSON line (NDJSON). `--path` selects a folder without changing the persisted working folder.

### `sn thumbnail <path>`

Fetch and display the 256×256 thumbnails for a file or folder, the same size Storage Navigator shows.

```bash
sn thumbnail mybucket/Library/scene.usd
sn thumbnail --json mybucket/Library/scene.usd   # thumbnail metadata as JSON
```

With `--json`, each entry has the thumbnail's `thumbnail.resourceAddress` and `thumbnail.resourceIdentity`, plus `createTime` and `specification` when the service returns them.

When the terminal supports inline image display, the thumbnail is rendered directly in the terminal. Otherwise, a `data:` URL is printed that can be pasted into a browser to view the image.

**Supported terminals for inline image display:**

| Terminal | Protocol                |
| -------- | ----------------------- |
| iTerm2   | iTerm2 (OSC 1337)       |
| WezTerm  | iTerm2 (OSC 1337)       |
| Ghostty  | Kitty Graphics Protocol |
| Kitty    | Kitty Graphics Protocol |

Kitty Graphics Protocol only supports PNG. Non-PNG thumbnails in a Kitty or Ghostty terminal fall back to a data URL.

To force the data URL even when inline rendering is available:

```bash
TERM_PROGRAM= sn thumbnail mybucket/Library/scene.usd
```

### `sn host services [name]`

List services available on a host. Defaults to the active host if no name is given.

```bash
sn host services
sn host services myhost
sn host services --json myhost
```

## Global Options

| Option              | Description                                                            |
| ------------------- | ---------------------------------------------------------------------- |
| `--json`            | Output as JSON                                                         |
| `--host <name>`     | Use a configured host for one command without changing the active host |
| `--non-interactive` | Fail if authentication needs a device login                            |
| `--verbose`         | Log HTTP requests, responses, and operation details to stderr          |
| `--no-color`        | Disable color output                                                   |

Finite commands write one JSON value with `--json`. Successful mutations return an object with `ok`, `command`, and path details. Errors write one JSON object to stderr and exit nonzero. Listings include `resourceAddress` for each item. Long-running notifications use NDJSON.
`--json` also disables interactive device login, so agent commands fail with `AUTH_REQUIRED` instead of waiting for a person. Run `sn host add` interactively first.

An HTTP 401 response can trigger one token refresh and retry. A repeated 401 fails the command; HTTP 403 permission failures return immediately without refreshing authentication.

`put`, `cp`, and `mv` default to `--on-conflict fail`. Use `skip` or `overwrite` explicitly when needed. `put`, `cp`, `mv`, and `rm` support `--dry-run` to preview remote changes. Copy and move use the storage API's atomic conflict flag; upload checks destination existence before writing, so another writer can still create the same path between the check and upload.

Permission management is not available in the CLI yet. The browser uses a separate private permission package whose API contract and ID-token requirements still need to be integrated. The `meta` command shows the resolved ACL metadata but does not grant or revoke permissions.

## Config

Config is stored at `~/.config/sn/config.json` and contains authentication tokens. Each write atomically replaces the file and enforces mode 0600 on POSIX systems; Windows access depends on the user profile's filesystem permissions. Config mutations merge into the latest state under a process lock. Token refreshes are serialized per host across processes, and a refresh response that omits a replacement refresh token retains the previous one.

### Transfer concurrency

`SN_CONCURRENCY` controls concurrent file workers and upload destination checks (default: 16). Set it to a positive integer, for example `SN_CONCURRENCY=4 sn get mybucket/Library/`. Folder traversal and stdin paths run in sequence so nested operations share the file worker limit rather than multiplying it by depth. Multipart uploads can upload up to 10 parts per file concurrently.

Direct and redirected uploads have no fixed request timeout, so large transfers can finish. Use Ctrl-C to interrupt a transfer. Failed multipart uploads do not finalize the object; the server may retain incomplete parts until its own cleanup policy removes them.

---

## Limitations

### Global Config File

`sn` stores all state — active host, working folder per host — in a single config file shared across all processes. This has two practical consequences:

**One persisted active host.** The active host is set globally with `sn host use <name>`. Concurrent processes can avoid switching it by passing `--host <name>` to each command.

**Concurrent instances conflict on relative paths.** The working folder is stored per host in the config file, not per shell session. Running multiple instances of `sn` concurrently using relative paths will share the same working folder state. Use full absolute paths to avoid this.

### Glob / Wildcard Patterns

Remote glob patterns are not supported. Use `sn --json ls` with `jq` to select resource addresses, then pipe them to `sn get` or `sn rm`, as shown above. Shell globs expand local files before `sn` receives arguments.

## Roadmap

The following features are planned for future releases:

- **User Info service support** — query user and group information from the identity service
- **Permission management** — add commands after the permission API contract and ID-token flow are verified

## Development

```bash
npm ci
npm run dev -- --help
npm run typecheck
npm run lint
npm test
npm run build
npm run test:package
npm audit
```

Tests use mocked storage services and a local HTTP server; they require no real storage credentials. The process tests exercise concurrent authentication and JSON output from the CLI. GitHub CI runs these checks on Node.js 22 and 24. The build uses an explicit patched esbuild override until upstream tools move to that version range.

## Development Notes

This tool was built using [Claude](https://claude.ai) (Anthropic) with the [Superpowers agentic skills framework](https://github.com/obra/superpowers), which provides structured workflows for AI-assisted software development.

The documents below are historical design and implementation notes. They may describe earlier behavior or proposed code; use this README and the current source for the supported interface.

### Superpowers artifacts in this repo

| Artifact             | Location                  | Purpose                                                                                            |
| -------------------- | ------------------------- | -------------------------------------------------------------------------------------------------- |
| Feature specs        | `docs/superpowers/specs/` | Design documents capturing requirements and behavior for each feature before implementation        |
| Implementation plans | `docs/superpowers/plans/` | Step-by-step execution plans generated from specs and used to guide Claude through multi-step work |
| Project skills       | `.agents/skills/`         | Reusable instructions for agents using the CLI                                                     |
