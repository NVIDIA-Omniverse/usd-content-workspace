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

import fs from "fs";
import path from "path";
import { fileURLToPath } from "url";
import react from "@vitejs/plugin-react-swc";
import { defineConfig } from "vite";
import tsconfigPaths from "vite-tsconfig-paths";

// @omniverse/storage-permission-ui is an optional dependency.
// Use a stub component if this feature is not installed, or is installed from
// sources whose library build (dist/) has not been produced yet.
function isPackageBuilt(dir) {
  try {
    const { main = "index.js" } = JSON.parse(fs.readFileSync(path.join(dir, "package.json"), "utf-8"));
    return fs.existsSync(path.join(dir, main));
  } catch {
    return false;
  }
}

const alias = [
  {
    // The package root also exports dynamic imports for every icon, causing
    // Vite's dev optimizer to emit thousands of chunks. Use static icons only.
    // https://github.com/tabler/tabler-icons/issues/1233
    find: /^@tabler\/icons-react$/,
    replacement: "@tabler/icons-react/dist/esm/icons/index.mjs",
  },
];
const permissionUiDir = "node_modules/@omniverse/storage-permission-ui";
if (!isPackageBuilt(permissionUiDir)) {
  if (fs.existsSync(permissionUiDir)) {
    console.warn(
      "@omniverse/storage-permission-ui is installed but not built; using the stub. " +
        "Run `npm run build:lib` in the package sources and reinstall to enable it.",
    );
  }
  alias.push({
    find: "@omniverse/storage-permission-ui",
    replacement: "src/stubs/@omniverse/storage-permission-ui.tsx",
  });
}

const serveAttributions = {
  name: "serve-attributions",
  configureServer(server) {
    server.middlewares.use("/ATTRIBUTIONS.md", (_req, res) => {
      res.setHeader("Content-Type", "text/markdown; charset=utf-8");
      res.end(fs.readFileSync("ATTRIBUTIONS.md", "utf-8"));
    });
  },
};

const pkg = JSON.parse(fs.readFileSync("package.json", "utf-8"));

// APP_VERSION carries the qualified publish version (e.g. 1.0.3-rc.11) injected by CI.
// Fall back to package.json for local development builds.
const appVersion = process.env.APP_VERSION || pkg.version;

export default defineConfig({
  plugins: [react(), tsconfigPaths(), serveAttributions],
  define: {
    __APP_VERSION__: JSON.stringify(appVersion),
  },
  resolve: {
    alias,
  },
  server: {},
  test: {
    globals: true,
    environment: "jsdom",
    setupFiles: "./vitest.setup.mjs",
    root: fileURLToPath(new URL("./", import.meta.url)),
    include: ["src/**/*.test.ts?(x)"],
    exclude: ["src/**/*.story.tsx"],
    coverage: {
      provider: "v8",
      include: ["src/**/*.tsx", "src/**/*.ts"],
      exclude: ["**/*.d.ts", "src/**/*.test.ts?(x)", "src/msw/*", "src/test-utils/*"],
    },
  },
});
