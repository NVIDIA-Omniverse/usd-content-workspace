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

import React, { memo } from "react";
import { Cell, flexRender, Row } from "@tanstack/react-table";
import { VirtualItem } from "@tanstack/react-virtual";
import { Table } from "@mantine/core";
import { Path } from "@/state/path";
import styles from "./PathContentTableRow.module.css";

export interface PathContentTableRowProps {
  row: Row<Path>;
  virtualItem: VirtualItem;
  onClick: (row: Row<Path>, event: React.MouseEvent) => void;
  onDoubleClick: (path: Row<Path>) => void;
  onContextMenu: (event: React.MouseEvent) => void;
}

export const PathContentTableRow = memo(
  ({ row, virtualItem, onClick, onDoubleClick, onContextMenu }: PathContentTableRowProps) => (
    <Table.Tr
      key={row.id}
      aria-label={row.original.name}
      bg={row.getIsSelected() ? "var(--mantine-color-blue-light)" : ""}
      className={styles.virtualizedRow}
      style={{
        transform: `translateY(${virtualItem.start}px)`,
      }}
      onClick={(event) => {
        event.stopPropagation();
        onClick(row, event);
      }}
      onDoubleClick={() => onDoubleClick(row)}
      onContextMenu={onContextMenu}
    >
      {row.getVisibleCells().map((cell: Cell<Path, unknown>) => {
        const isNameColumn = cell.column.id === "resourceAddress";
        return (
          <Table.Td
            key={cell.id}
            className={`${styles.tableCell} ${isNameColumn ? styles.expandingTableCell : ""}`}
            style={{ width: isNameColumn ? undefined : cell.column.getSize() }}
          >
            {flexRender(cell.column.columnDef.cell, cell.getContext())}
          </Table.Td>
        );
      })}
    </Table.Tr>
  )
);

PathContentTableRow.displayName = "VirtualizedRow";
