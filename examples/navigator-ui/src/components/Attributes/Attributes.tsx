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
import { Table, Text } from "@mantine/core";
import styles from "./Attributes.module.css";

export interface AttributesProps {
  attributes: Attribute[];
}

interface Attribute {
  title: string;
  value?: ReactNode;
}

export function Attributes({ attributes }: AttributesProps) {
  return (
    <Table className={styles.attrs} variant="vertical">
      <Table.Tbody>
        {attributes.map((attribute) => (
          <Table.Tr key={attribute.title}>
            <Table.Th w={160} px="md" py="xs">
              <Text size="xs">{attribute.title}</Text>
            </Table.Th>
            <Table.Td>
              <Text size="xs">{attribute.value}</Text>
            </Table.Td>
          </Table.Tr>
        ))}
      </Table.Tbody>
    </Table>
  );
}
