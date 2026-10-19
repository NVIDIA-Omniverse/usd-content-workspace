/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
import {
  CodeHighlightAdapterProvider,
  createHighlightJsAdapter,
} from "@mantine/code-highlight";
import {
  createTheme,
  type MantineColorsTuple,
  MantineProvider,
} from "@mantine/core";
import { Notifications } from "@mantine/notifications";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import hljs, { type LanguageFn } from "highlight.js";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { RouterProvider } from "react-router-dom";
import ConfigProvider from "./providers/ConfigProvider";
import { router } from "./router";

import "@mantine/core/styles.layer.css";
import "@mantine/code-highlight/styles.layer.css";
import "@mantine/notifications/styles.layer.css";
import "highlight.js/styles/a11y-dark.min.css";

const myColor: MantineColorsTuple = [
  "#f1f1ff",
  "#e0dff2",
  "#bfbdde",
  "#9b98ca",
  "#7d79b9",
  "#6a66af",
  "#605cac",
  "#504c97",
  "#464388",
  "#3b3979",
];

const theme = createTheme({
  fontFamily: "Rubik, sans-serif",
  colors: {
    myColor,
  },
  fontSizes: {
    xs: "9pt",
    sm: "10pt",
    md: "13px",
    lg: "14pt",
    xl: "16pt",
  },
  headings: {
    sizes: {
      h1: { fontSize: "24pt" },
      h2: { fontSize: "18pt" },
      h3: { fontSize: "15pt" },
      h4: { fontSize: "12pt" },
      h5: { fontSize: "11pt" },
      h6: { fontSize: "9pt" },
    },
  },
});

hljs.registerLanguage("cedar", hljsCedar);

const highlightAdapter = createHighlightJsAdapter(hljs);

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <QueryClientProvider client={new QueryClient()}>
      <MantineProvider theme={theme} forceColorScheme={"dark"}>
        <CodeHighlightAdapterProvider adapter={highlightAdapter}>
          <ConfigProvider>
            <RouterProvider router={router} />
            <Notifications />
          </ConfigProvider>
        </CodeHighlightAdapterProvider>
      </MantineProvider>
    </QueryClientProvider>
  </StrictMode>,
);

declare global {
  const hljsCedar: LanguageFn;
}
