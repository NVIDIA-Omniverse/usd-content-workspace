#!/usr/bin/env node
// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
import chalk from "chalk";
import { Command, CommanderError } from "commander";
import { runCd } from "@/commands/cd";
import { runCp } from "@/commands/cp";
import { runGet, runGetLines } from "@/commands/get";
import {
  addHost,
  listHosts,
  listServices,
  removeHost,
  useHost,
} from "@/commands/host";
import { runLs } from "@/commands/ls";
import { runMkdir } from "@/commands/mkdir";
import { runMv } from "@/commands/mv";
import { runPut, runPutLines } from "@/commands/put";
import { runRm, runRmLines } from "@/commands/rm";
import { runMeta } from "@/commands/meta";
import { runTags } from "@/commands/tags";
import { runArchive } from "@/commands/archive";
import { runVersionsList, runVersionsRestore } from "@/commands/versions";
import { runNotification } from "@/commands/notification";
import { runThumbnail } from "@/commands/thumbnail";
import { buildContext, getDiscoveredApi } from "@/context";
import { create } from "@/services/api";
import { setDeviceFlowAllowed } from "@/services/auth";
import { capabilitiesService } from "@/services/storage/capabilitiesService";
import {
  noopLogger,
  CliError,
  printError,
  printJson,
  printServices,
  printSuccess,
  setCommandName,
  setJsonMode,
  verboseLogger,
} from "@/output";
import { isStdinPiped, readStdinLines } from "@/util/stdin";
import { resolveStoragePath } from "@/util/resolve";
import { ensureFolderPath } from "@/util/url";
import { parseConflictPolicy } from "@/util/mutation";

function globalOpts(cmd: Command): {
  json?: boolean;
  verbose?: boolean;
  host?: string;
} {
  return cmd.optsWithGlobals();
}

const program = new Command();
program.configureOutput({
  writeErr: (message) => {
    if (!process.argv.includes("--json")) process.stderr.write(message);
  },
});
program.exitOverride();
program
  .name("sn")
  .description("Storage Navigator CLI")
  .version("0.1.1")
  .option("--json", "output as JSON", false)
  .option("--host <name>", "use a configured host for this command")
  .option("--non-interactive", "fail instead of starting a device login", false)
  .option(
    "--verbose",
    "log HTTP requests, responses, and operation details to stderr",
    false,
  )
  .option("--no-color", "disable color output");
program.hook("preAction", (_, actionCommand) => {
  const opts = actionCommand.optsWithGlobals();
  setJsonMode(Boolean(opts.json));
  setDeviceFlowAllowed(!opts.json && !opts.nonInteractive);
  const names: string[] = [];
  for (
    let command: Command | null = actionCommand;
    command?.parent;
    command = command.parent
  ) {
    names.unshift(command.name());
  }
  setCommandName(names.join(" "));
});

// host
const host = program.command("host").description("manage storage hosts");

host
  .command("add <name> <discovery-url>")
  .description("add a host and authenticate via device flow")
  .action(async (name: string, discoveryUrl: string) => {
    try {
      await addHost(name, discoveryUrl);
      printSuccess(`Host '${name}' added and set as active.`, { host: name });
    } catch (e) {
      printError(e);
      process.exit(1);
    }
  });

host
  .command("use <name>")
  .description("switch active host")
  .action(async (name: string) => {
    try {
      await useHost(name);
      printSuccess(`Now using '${name}'.`, { host: name });
    } catch (e) {
      printError(e);
      process.exit(1);
    }
  });

host
  .command("list")
  .description("list configured hosts")
  .action(() => {
    const opts = globalOpts(program);
    const hosts = listHosts();
    if (opts.json) {
      printJson(hosts);
      return;
    }
    if (hosts.length === 0) {
      process.stdout.write("No hosts configured. Run 'sn host add'.\n");
      return;
    }
    for (const h of hosts) {
      const marker = h.active ? chalk.green("* ") : "  ";
      const name = h.active ? chalk.green(h.name) : h.name;
      process.stdout.write(`${marker}${name}  ${h.discoveryUrl}\n`);
    }
  });

host
  .command("remove <name>")
  .description("remove a host")
  .action(async (name: string) => {
    try {
      await removeHost(name);
      printSuccess(`Host '${name}' removed.`, { host: name });
    } catch (e) {
      printError(e);
      process.exit(1);
    }
  });

host
  .command("services [name]")
  .description("list services available on a host")
  .action(async (name: string | undefined) => {
    try {
      const services = await listServices(name ?? globalOpts(program).host);
      if (globalOpts(program).json) {
        printJson(services);
      } else {
        printServices(services);
      }
    } catch (e) {
      printError(e);
      process.exit(1);
    }
  });

// cd
program
  .command("cd [path]")
  .description("set current working folder (no argument clears it)")
  .action(async (path: string | undefined) => {
    try {
      const { api, hostName, verbose } = buildContext(globalOpts(program));
      const log = verbose ? verboseLogger() : noopLogger();
      await runCd(api, path, hostName, log);
    } catch (e) {
      printError(e);
      process.exit(1);
    }
  });

program
  .command("pwd")
  .description("print current working folder")
  .action(() => {
    try {
      const opts = globalOpts(program);
      const { cwd } = buildContext(opts);
      if (opts.json) {
        printJson({ cwd: cwd ?? null });
      } else {
        process.stdout.write(
          cwd ? `${cwd}\n` : "No current folder set. Run 'sn cd' to set one.\n",
        );
      }
    } catch (e) {
      printError(e);
      process.exit(1);
    }
  });

// file commands
program
  .command("ls [path]")
  .description("list folder contents")
  .option("-l", "long format with size and date")
  .action(async (path: string | undefined, cmdOpts: { l: boolean }) => {
    try {
      const { api, json, cwd, verbose } = buildContext(globalOpts(program));
      const log = verbose ? verboseLogger() : noopLogger();
      await runLs(api, path, { long: cmdOpts.l, json }, log, cwd);
    } catch (e) {
      printError(e);
      process.exit(1);
    }
  });

program
  .command("meta <path>")
  .description("show file or folder metadata, including ACL and tags")
  .action(async (path: string) => {
    try {
      const { api, json, cwd } = buildContext(globalOpts(program));
      await runMeta(api, path, { json }, cwd);
    } catch (e) {
      printError(e);
      process.exit(1);
    }
  });

const tags = program
  .command("tags")
  .description("read or update file and folder tags");
tags
  .command("list <path>")
  .description("list tags and their ETag")
  .action(async (path: string) => {
    try {
      const { api, json, cwd } = buildContext(globalOpts(program));
      await runTags(api, "list", path, { json }, cwd);
    } catch (e) {
      printError(e);
      process.exit(1);
    }
  });
tags
  .command("set <path> <key> <value>")
  .description("set a tag using its current ETag")
  .action(async (path: string, key: string, value: string) => {
    try {
      const { api, json, cwd } = buildContext(globalOpts(program));
      await runTags(api, "set", path, { key, value, json }, cwd);
    } catch (e) {
      printError(e);
      process.exit(1);
    }
  });
tags
  .command("remove <path> <key>")
  .description("remove a tag using its current ETag")
  .action(async (path: string, key: string) => {
    try {
      const { api, json, cwd } = buildContext(globalOpts(program));
      await runTags(api, "remove", path, { key, json }, cwd);
    } catch (e) {
      printError(e);
      process.exit(1);
    }
  });

program
  .command("archive <paths...>")
  .description("download files and folders as a server-generated ZIP")
  .requiredOption("-o, --output <file>", "local ZIP destination")
  .option("--overwrite", "replace an existing local file", false)
  .action(
    async (paths: string[], opts: { output: string; overwrite: boolean }) => {
      try {
        await runArchive(
          buildContext(globalOpts(program)),
          paths,
          opts.output,
          opts.overwrite,
        );
      } catch (e) {
        printError(e);
        process.exit(1);
      }
    },
  );

program
  .command("get [remote-path] [local-dest]")
  .description("download a file or folder")
  .option("--progress", "show transfer progress", false)
  .option("--overwrite", "overwrite existing local file", false)
  .action(
    async (
      remotePath: string | undefined,
      localDest: string | undefined,
      cmdOpts: { progress: boolean; overwrite: boolean }, // Commander injects options after positional args
    ) => {
      try {
        const { api, cwd, verbose } = buildContext(globalOpts(program));
        const log = verbose ? verboseLogger() : noopLogger();
        if (remotePath === undefined) {
          if (!isStdinPiped()) {
            printError("remote-path required");
            process.exit(1);
          }
          const lines = await readStdinLines();
          if (lines.length === 0) {
            printSuccess("Downloaded 0 paths", { paths: 0 });
            return;
          }
          await runGetLines(
            api,
            lines,
            localDest,
            log,
            cwd,
            cmdOpts.progress && !verbose,
            cmdOpts.overwrite,
          );
          return;
        }
        await runGet(
          api,
          remotePath,
          localDest,
          log,
          cwd,
          cmdOpts.progress && !verbose,
          cmdOpts.overwrite,
        );
      } catch (e) {
        printError(e);
        process.exit(1);
      }
    },
  );

program
  .command("put [local-path] [remote-path]")
  .description(
    "upload a file or folder (defaults to local filename in current folder)",
  )
  .option("--progress", "show transfer progress", false)
  .option("--dry-run", "preview uploads without writing", false)
  .option(
    "--on-conflict <policy>",
    "fail, skip, or overwrite existing remote files",
    "fail",
  )
  .action(
    async (
      localPath: string | undefined,
      remotePath: string | undefined,
      cmdOpts: { progress: boolean; dryRun: boolean; onConflict: string }, // Commander injects options after positional args
    ) => {
      try {
        const { api, cwd, verbose } = buildContext(globalOpts(program));
        const log = verbose ? verboseLogger() : noopLogger();
        if (localPath === undefined) {
          if (!isStdinPiped()) {
            printError("local-path required");
            process.exit(1);
          }
          if (remotePath !== undefined) {
            printError("remote-path cannot be used when reading from stdin");
            process.exit(1);
          }
          const lines = await readStdinLines();
          if (lines.length === 0) {
            printSuccess("Uploaded 0 paths", { paths: 0 });
            return;
          }
          await runPutLines(
            api,
            lines,
            log,
            cwd,
            cmdOpts.progress && !verbose,
            {
              dryRun: cmdOpts.dryRun,
              onConflict: parseConflictPolicy(cmdOpts.onConflict),
            },
          );
          return;
        }
        await runPut(
          api,
          localPath,
          remotePath,
          log,
          cwd,
          cmdOpts.progress && !verbose,
          {
            dryRun: cmdOpts.dryRun,
            onConflict: parseConflictPolicy(cmdOpts.onConflict),
          },
        );
      } catch (e) {
        printError(e);
        process.exit(1);
      }
    },
  );

program
  .command("mkdir <path>")
  .description("create a folder")
  .action(async (path: string) => {
    try {
      const { api, cwd, verbose } = buildContext(globalOpts(program));
      const log = verbose ? verboseLogger() : noopLogger();
      await runMkdir(api, path, log, cwd);
    } catch (e) {
      printError(e);
      process.exit(1);
    }
  });

program
  .command("rm [path]")
  .description("delete a file or folder (-r for non-empty folders)")
  .option("-r", "recursive delete for non-empty folders")
  .option("--dry-run", "preview deletions without writing", false)
  .action(
    async (
      path: string | undefined,
      cmdOpts: { r: boolean; dryRun: boolean },
    ) => {
      try {
        const { api, cwd, verbose } = buildContext(globalOpts(program));
        const log = verbose ? verboseLogger() : noopLogger();
        if (path === undefined) {
          if (!isStdinPiped()) {
            printError("path required");
            process.exit(1);
          }
          const lines = await readStdinLines();
          if (lines.length === 0) {
            printSuccess("Deleted 0 paths", { paths: 0 });
            return;
          }
          await runRmLines(
            api,
            lines,
            { recursive: cmdOpts.r, dryRun: cmdOpts.dryRun },
            log,
            cwd,
          );
          return;
        }
        await runRm(
          api,
          path,
          { recursive: cmdOpts.r, dryRun: cmdOpts.dryRun },
          log,
          cwd,
        );
      } catch (e) {
        printError(e);
        process.exit(1);
      }
    },
  );

program
  .command("mv <src> <dst>")
  .description("move or rename a file")
  .option("--dry-run", "preview moves without writing", false)
  .option(
    "--on-conflict <policy>",
    "fail, skip, or overwrite existing remote files",
    "fail",
  )
  .action(
    async (
      src: string,
      dst: string,
      cmdOpts: { dryRun: boolean; onConflict: string },
    ) => {
      try {
        const { api, cwd, verbose } = buildContext(globalOpts(program));
        const log = verbose ? verboseLogger() : noopLogger();
        await runMv(api, src, dst, log, cwd, {
          dryRun: cmdOpts.dryRun,
          onConflict: parseConflictPolicy(cmdOpts.onConflict),
        });
      } catch (e) {
        printError(e);
        process.exit(1);
      }
    },
  );

program
  .command("cp <src> <dst>")
  .description("copy a file")
  .option("--dry-run", "preview copies without writing", false)
  .option(
    "--on-conflict <policy>",
    "fail, skip, or overwrite existing remote files",
    "fail",
  )
  .action(
    async (
      src: string,
      dst: string,
      cmdOpts: { dryRun: boolean; onConflict: string },
    ) => {
      try {
        const { api, cwd, verbose } = buildContext(globalOpts(program));
        const log = verbose ? verboseLogger() : noopLogger();
        await runCp(api, src, dst, log, cwd, {
          dryRun: cmdOpts.dryRun,
          onConflict: parseConflictPolicy(cmdOpts.onConflict),
        });
      } catch (e) {
        printError(e);
        process.exit(1);
      }
    },
  );

// versions
const versions = program
  .command("versions")
  .description("list and restore file versions");

versions
  .command("list <path>")
  .description("list available versions of a file")
  .action(async (path: string) => {
    try {
      const { api, json, cwd, verbose } = buildContext(globalOpts(program));
      const log = verbose ? verboseLogger() : noopLogger();
      await runVersionsList(api, path, { json }, log, cwd);
    } catch (e) {
      printError(e);
      process.exit(1);
    }
  });

versions
  .command("restore <path> <identity>")
  .description("restore a version in-place")
  .action(async (path: string, identity: string) => {
    try {
      const { api, cwd, verbose } = buildContext(globalOpts(program));
      const log = verbose ? verboseLogger() : noopLogger();
      await runVersionsRestore(api, path, identity, log, cwd);
    } catch (e) {
      printError(e);
      process.exit(1);
    }
  });

// notification
program
  .command("notification <types...>")
  .description("watch for notification events in a folder")
  .option("--path <path>", "folder to watch (defaults to current folder)")
  .action(async (types: string[], cmdOpts: { path?: string }) => {
    try {
      const { api, cwd, json, verbose, hostName, discoveryUrl } = buildContext(
        globalOpts(program),
      );
      const log = verbose ? verboseLogger() : noopLogger();
      const endpoint = await getDiscoveredApi(api, "notifications-consumer");
      if (!endpoint)
        throw new Error("notification service is not available on this host");
      const notificationApi = create(endpoint.baseUrl, {
        hostName,
        discovery: discoveryUrl,
        verbose,
        route: (url) => {
          const prefix = endpoint.baseUrl.replace(/\/$/, "") + "/api/v1beta/";
          return url.startsWith(prefix)
            ? `${endpoint.url.replace(/\/$/, "")}/${url.slice(prefix.length)}`
            : null;
        },
      });
      const watchPath = cmdOpts.path
        ? ensureFolderPath(
            await resolveStoragePath(
              capabilitiesService(api),
              cmdOpts.path,
              cwd,
            ),
          )
        : cwd;
      await runNotification(notificationApi, watchPath, types, { json }, log);
    } catch (e) {
      printError(e);
      process.exit(1);
    }
  });

// thumbnail
program
  .command("thumbnail <path>")
  .description("fetch thumbnail information for a file or folder")
  .action(async (path: string) => {
    try {
      const { api, json, cwd, verbose, hostName, discoveryUrl } = buildContext(
        globalOpts(program),
      );
      const endpoint = await getDiscoveredApi(api, "thumbnail");
      if (!endpoint)
        throw new Error("thumbnail service is not available on this host");
      const thumbApi = create(endpoint.baseUrl, {
        hostName,
        discovery: discoveryUrl,
        verbose,
        route: (url) => {
          const prefix = endpoint.baseUrl.replace(/\/$/, "") + "/v1alpha/";
          return url.startsWith(prefix)
            ? `${endpoint.url.replace(/\/$/, "")}/${url.slice(prefix.length)}`
            : null;
        },
      });
      await runThumbnail(api, thumbApi, path, { json }, cwd);
    } catch (e) {
      printError(e);
      process.exit(1);
    }
  });

if (process.argv.includes("--no-color")) {
  process.env.NO_COLOR = "1";
}

try {
  await program.parseAsync(process.argv);
} catch (error) {
  if (error instanceof CommanderError) {
    if (error.exitCode === 0) process.exit(0);
    setJsonMode(process.argv.includes("--json"));
    if (process.argv.includes("--json"))
      printError(new CliError(error.message, "USAGE_ERROR"));
    process.exit(error.exitCode);
  }
  setJsonMode(process.argv.includes("--json"));
  printError(error);
  process.exit(1);
}
