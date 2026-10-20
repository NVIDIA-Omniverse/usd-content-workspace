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
import { Box, Group } from "@mantine/core";
import { ModalsProvider } from "@mantine/modals";
import type { ReactNode } from "react";
import { Outlet } from "react-router-dom";
import { AuthRequired } from "../components/AuthRequired";
import { Navbar } from "../components/Navbar";
import { AuthProvider } from "../providers/AuthProvider";
import styles from "./Main.module.css";

export interface MainProps {
  children?: ReactNode;
}

export function Main({ children }: MainProps) {
  return (
    <AuthProvider>
      <ModalsProvider>
        <AuthRequired>
          <Group
            align={"start"}
            justify={"stretch"}
            gap={0}
            wrap={"nowrap"}
            h={"100vh"}
          >
            <Navbar />
            <Box component={"main"} className={styles.outlet}>
              <Outlet />
            </Box>
            {children}
          </Group>
        </AuthRequired>
      </ModalsProvider>
    </AuthProvider>
  );
}
