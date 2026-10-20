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

import React from "react";
import { PolymorphicComponentProps, Text, TextProps } from "@mantine/core";
import styles from "./PathText.module.css";

export interface PathTextProps extends PolymorphicComponentProps<"p", TextProps> {
  children?: React.ReactNode;
}

export function PathText({ children, ...props }: PathTextProps) {
  return (
    <Text className={styles.path} c="dimmed" size="xs" {...props}>
      {children}
    </Text>
  );
}
