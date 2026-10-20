// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, beforeAll, expect, it } from "vitest";

const address = "omniverse://nucleus/Library/scene.usd";
let protectedDiscovery = false;
let refreshCalls = 0;
const server = createServer((request, response) => {
  if (request.url === "/api/v1/auth-config") {
    response.setHeader("Content-Type", "application/json");
    response.end(
      JSON.stringify({
        openid_configuration: `${serverUrl}/oidc`,
        clients: { navigator: { client_id: "cli", scope: "openid" } },
      }),
    );
    return;
  }
  if (request.url === "/oidc") {
    response.setHeader("Content-Type", "application/json");
    response.end(
      JSON.stringify({
        token_endpoint: `${serverUrl}/token`,
        authorization_endpoint: `${serverUrl}/authorize`,
      }),
    );
    return;
  }
  if (request.url === "/token") {
    refreshCalls++;
    let body = "";
    request.on("data", (chunk: Buffer) => {
      body += chunk.toString();
    });
    request.on("end", () => {
      response.setHeader("Content-Type", "application/json");
      if (
        refreshCalls > 1 ||
        new URLSearchParams(body).get("refresh_token") !== "single-use"
      ) {
        response.statusCode = 400;
        response.end(JSON.stringify({ error: "invalid_grant" }));
      } else {
        setTimeout(
          () =>
            response.end(
              JSON.stringify({
                access_token: "new-access",
                refresh_token: "rotated",
                id_token: "id",
                expires_in: 3600,
              }),
            ),
          100,
        );
      }
    });
    return;
  }
  if (
    (request.url === "/api/v2/services" ||
      request.url === "/api/v1/services") &&
    protectedDiscovery &&
    request.headers.authorization !== "Bearer new-access"
  ) {
    response.statusCode = 401;
    response.end();
    return;
  }
  if (request.url === "/api/v2/services") {
    response.setHeader("Content-Type", "application/json");
    response.end(
      JSON.stringify({
        "schema-version": 2,
        services: [
          {
            name: "storage",
            rest: {
              url: serverUrl,
              endpoints: [
                {
                  name: "metadata",
                  versions: { v1alpha: { path: "/v1alpha" } },
                },
              ],
            },
          },
        ],
      }),
    );
    return;
  }
  if (
    request.method === "POST" &&
    request.url?.startsWith("/v1alpha/metadata/")
  ) {
    response.setHeader("Content-Type", "application/json");
    response.end(
      JSON.stringify({ tags: { etag: "tag-etag", value: { owner: "team" } } }),
    );
    return;
  }
  if (request.method === "PUT" && request.url?.endsWith("/tags")) {
    response.statusCode = 412;
    response.end();
    return;
  }
  response.statusCode = 404;
  response.end();
});
let serverUrl: string;
let home: string;

beforeAll(async () => {
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const addressInfo = server.address();
  if (!addressInfo || typeof addressInfo === "string")
    throw new Error("missing server address");
  serverUrl = `http://127.0.0.1:${addressInfo.port}`;
  home = await mkdtemp(join(tmpdir(), "sn-process-"));
  await mkdir(join(home, ".config", "sn"), { recursive: true });
  await writeFile(
    join(home, ".config", "sn", "config.json"),
    JSON.stringify({
      activeHost: "local",
      hosts: {
        local: { discoveryUrl: serverUrl, storageUrl: serverUrl, authUrl: "" },
      },
    }),
  );
});

afterAll(async () => {
  await new Promise<void>((resolve) => server.close(() => resolve()));
  await rm(home, { recursive: true, force: true });
});

async function run(
  args: string[],
): Promise<{ code: number | null; stdout: string; stderr: string }> {
  const child = spawn(
    process.execPath,
    [
      join(process.cwd(), "node_modules/tsx/dist/cli.mjs"),
      "src/index.ts",
      ...args,
    ],
    {
      cwd: process.cwd(),
      env: { ...process.env, HOME: home },
    },
  );
  let stdout = "";
  let stderr = "";
  child.stdout.setEncoding("utf8").on("data", (chunk: string) => {
    stdout += chunk;
  });
  child.stderr.setEncoding("utf8").on("data", (chunk: string) => {
    stderr += chunk;
  });
  return new Promise((resolve, reject) => {
    child.on("error", reject);
    child.on("close", (code) => resolve({ code, stdout, stderr }));
  });
}

it("keeps success and failure JSON on their documented streams", async () => {
  const listed = await run(["--json", "tags", "list", address]);
  expect(listed.code).toBe(0);
  expect(listed.stderr).toBe("");
  expect(JSON.parse(listed.stdout)).toEqual({
    path: address,
    etag: "tag-etag",
    tags: { owner: "team" },
  });

  const conflict = await run([
    "--json",
    "tags",
    "set",
    address,
    "owner",
    "other",
  ]);
  expect(conflict.code).not.toBe(0);
  expect(conflict.stdout).toBe("");
  expect(JSON.parse(conflict.stderr)).toMatchObject({
    ok: false,
    error: { code: "CONFLICT", status: 412 },
  });

  const missingHost = await run([
    "--json",
    "--host",
    "missing",
    "tags",
    "list",
    address,
  ]);
  expect(missingHost.code).not.toBe(0);
  expect(missingHost.stdout).toBe("");
  expect(JSON.parse(missingHost.stderr)).toMatchObject({
    ok: false,
    error: { code: "HOST_NOT_FOUND" },
  });
});

it("coordinates rotating refresh tokens across CLI processes", async () => {
  const configFile = join(home, ".config", "sn", "config.json");
  const original = await readFile(configFile, "utf8");
  try {
    await writeFile(
      configFile,
      JSON.stringify({
        activeHost: "local",
        hosts: {
          local: {
            discoveryUrl: serverUrl,
            storageUrl: serverUrl,
            authUrl: "",
            accessToken: "expired",
            refreshToken: "single-use",
            tokenExpiry: new Date(0).toISOString(),
          },
        },
      }),
    );
    protectedDiscovery = true;
    const results = await Promise.all(
      Array.from({ length: 4 }, () => run(["--json", "tags", "list", address])),
    );
    expect(results.map(({ code, stderr }) => ({ code, stderr }))).toEqual(
      Array.from({ length: 4 }, () => ({ code: 0, stderr: "" })),
    );
    expect(refreshCalls).toBe(1);
    expect(JSON.parse(await readFile(configFile, "utf8"))).toMatchObject({
      hosts: { local: { accessToken: "new-access", refreshToken: "rotated" } },
    });
  } finally {
    protectedDiscovery = false;
    await writeFile(configFile, original);
  }
}, 15000);

it("writes one JSON error for corrupt configuration, including host list", async () => {
  const configFile = join(home, ".config", "sn", "config.json");
  const original = await readFile(configFile, "utf8");
  try {
    await writeFile(configFile, "{invalid");
    const result = await run(["--json", "host", "list"]);
    expect(result.code).toBe(1);
    expect(result.stdout).toBe("");
    expect(JSON.parse(result.stderr)).toMatchObject({
      ok: false,
      error: { code: "COMMAND_FAILED" },
    });
  } finally {
    await writeFile(configFile, original);
  }
});
