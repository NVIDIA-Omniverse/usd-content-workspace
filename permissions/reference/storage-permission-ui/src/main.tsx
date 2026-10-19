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

import {
  createTheme,
  type MantineColorsTuple,
  MantineProvider,
} from "@mantine/core";
import { Notifications } from "@mantine/notifications";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { Example } from "./examples/Example.js";

import "@mantine/core/styles.layer.css";
import "@mantine/notifications/styles.layer.css";

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
});

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <MantineProvider theme={theme} forceColorScheme={"dark"}>
      <Notifications />
      <QueryClientProvider client={new QueryClient()}>
        <Example />
      </QueryClientProvider>
    </MantineProvider>
  </StrictMode>,
);
