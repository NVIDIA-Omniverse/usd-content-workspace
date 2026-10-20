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

import { ReactNode } from "react";
import { IconInfoCircle } from "@tabler/icons-react";
import { Alert, AlertVariant, Box, MantineColor } from "@mantine/core";

export interface LoaderErrorProps {
  color?: MantineColor;
  icon?: ReactNode;
  title?: string;
  variant?: AlertVariant;
  children?: ReactNode;
}

export function LoaderError({
  color = "red",
  icon = <IconInfoCircle />,
  title = "Error",
  variant = "filled",
  children,
}: LoaderErrorProps) {
  return (
    <Box p="xl" pos="relative" style={{ zIndex: 1 }}>
      <Alert variant={variant} color={color} title={title} icon={icon}>
        {children}
      </Alert>
    </Box>
  );
}
