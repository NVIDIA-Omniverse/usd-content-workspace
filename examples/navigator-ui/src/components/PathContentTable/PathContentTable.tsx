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

import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { IconBucket, IconFile, IconFolder } from "@tabler/icons-react";
import {
  ColumnDef,
  flexRender,
  getCoreRowModel,
  HeaderGroup,
  Row,
  RowSelectionState,
  useReactTable,
} from "@tanstack/react-table";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useNavigate } from "react-router-dom";
import { Box, Checkbox, Table } from "@mantine/core";
import { UploadDropzone } from "@/components/UploadDropzone/UploadDropzone";
import { VirtualContainer } from "@/components/VirtualContainer";
import { usePathContextMenu } from "@/hooks/usePathContextMenu";
import { constructPathNavLink } from "@/hooks/usePathNavigation";
import { usePathSelectionWithModifiers } from "@/hooks/usePathSelectionWithModifiers";
import { useAppStore } from "@/state/appStore";
import { Path, PathType } from "@/state/path";
import { Connection } from "@/state/slices/connectionSlice";
import { PathContentTableRow } from "./PathContentTableRow";
import styles from "./PathContentTable.module.css";

export interface PathContentTableProps {
  connection: Connection;
  data: Path[];
}

export function PathContentTable({ connection, data }: PathContentTableProps) {
  const [rowSelection, setRowSelection] = useState<RowSelectionState>({});
  const setSelectedPaths = useAppStore((state) => state.setSelectedPaths);
  const selectedPaths = useAppStore((state) => state.connections[connection.endpoint]?.selectedPaths);

  const pathByAddress = useMemo(() => new Map(data.map((path) => [path.resourceAddress, path])), [data]);

  useEffect(() => {
    if (selectedPaths && data.length > 0) {
      const selectedIds = new Set([...selectedPaths].map((p) => p.resourceAddress));
      const initialSelection: RowSelectionState = {};
      for (const path of data) {
        if (selectedIds.has(path.resourceAddress)) {
          initialSelection[path.resourceAddress] = true;
        }
      }
      setRowSelection(initialSelection);
    }
  }, [selectedPaths, data]);

  const table = useReactTable({
    data,
    columns: tableColumnsDef,
    getCoreRowModel: getCoreRowModel(),
    getRowId: (row) => row.resourceAddress,
    onRowSelectionChange: (updater) => {
      const newRowSelection = typeof updater === "function" ? updater(rowSelection) : updater;
      setRowSelection(newRowSelection);
      const paths: Path[] = [];
      for (const key of Object.keys(newRowSelection)) {
        if (newRowSelection[key]) {
          const path = pathByAddress.get(key);
          if (path) {
            paths.push(path);
          }
        }
      }
      setSelectedPaths(connection.endpoint, paths);
    },
    state: {
      rowSelection,
    },
  });

  useEffect(() => {
    table.reset();
  }, [connection.currentFolder, table]);

  const parentRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: table.getRowModel().rows.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 35,
    overscan: 10,
  });

  const handlePathClick = usePathSelectionWithModifiers(
    data,
    selectedPaths,
    useCallback(
      (paths: Path[]) => {
        const rowSelection: RowSelectionState = {};
        paths.forEach((p) => {
          rowSelection[p.resourceAddress] = true;
        });
        table.setRowSelection(rowSelection);
      },
      [table]
    )
  );

  const handleRowClick = useCallback(
    (row: Row<Path>, event: React.MouseEvent) => {
      handlePathClick(row.original, event);
    },
    [handlePathClick]
  );

  const navigate = useNavigate();
  const handleRowDoubleClick = useCallback(
    (row: Row<Path>) => {
      if (row.original.isFolderLike) {
        void navigate(constructPathNavLink(connection.endpoint, row.original.resourceAddress));
      }
    },
    [connection.endpoint, navigate]
  );

  const showPathContextMenu = usePathContextMenu(connection);
  const handleContextMenu = useCallback(
    (row: Row<Path>) => (event: React.MouseEvent) => {
      const selection = table.getSelectedRowModel();
      const selectedPaths = selection.rows.map((row) => row.original);

      const displayMultiSelectMenu = row.getIsSelected() && selection.rows.length > 1;
      if (!displayMultiSelectMenu) {
        table.setRowSelection({
          [row.id]: true,
        });
      }

      void showPathContextMenu(displayMultiSelectMenu ? selectedPaths : [row.original])(event);
    },
    [table, showPathContextMenu]
  );

  const handleEmptyContentClick = useCallback(() => {
    if (selectedPaths?.size) {
      setSelectedPaths(connection.endpoint, []);
    }
  }, [connection.endpoint, selectedPaths?.size, setSelectedPaths]);

  return (
    <VirtualContainer containerRef={parentRef} aria-label="Folder content" onClick={handleEmptyContentClick}>
      <UploadDropzone connection={connection} className={styles.tableDropzone}>
        <Table stickyHeader highlightOnHover width="100%" className={styles.tableWrapper}>
          <TableHeader headerGroups={table.getHeaderGroups()} />
          <Table.Tbody
            className={styles.tbodyGrid}
            style={{
              height: `${virtualizer.getTotalSize()}px`,
            }}
          >
            {virtualizer.getVirtualItems().map((virtualItem) => {
              const row = table.getRowModel().rows[virtualItem.index];
              return (
                <PathContentTableRow
                  key={row.id}
                  row={row}
                  virtualItem={virtualItem}
                  onClick={handleRowClick}
                  onDoubleClick={handleRowDoubleClick}
                  onContextMenu={handleContextMenu(row)}
                />
              );
            })}
          </Table.Tbody>
        </Table>
      </UploadDropzone>
    </VirtualContainer>
  );
}

function TableHeader({ headerGroups }: { headerGroups: HeaderGroup<Path>[] }) {
  return (
    <Table.Thead onClick={(event) => event.stopPropagation()}>
      {headerGroups.map((headerGroup) => (
        <Table.Tr key={headerGroup.id} className={styles.headerRow}>
          {headerGroup.headers.map((header) => {
            const isNameColumn = header.column.id === "resourceAddress";
            return (
              <Table.Th
                key={header.id}
                className={`${styles.headerCell} ${isNameColumn ? styles.expandingHeaderCell : ""}`}
                style={{ width: isNameColumn ? undefined : header.getSize() }}
              >
                {flexRender(header.column.columnDef.header, header.getContext())}
              </Table.Th>
            );
          })}
        </Table.Tr>
      ))}
    </Table.Thead>
  );
}

const tableColumnsDef: ColumnDef<Path>[] = [
  {
    id: "isSelected",
    header: ({ table }) => (
      <Box component="label" className={styles.checkboxHitbox} htmlFor="path-table-select-all">
        <Checkbox
          id="path-table-select-all"
          title="Select all rows"
          checked={table.getIsAllPageRowsSelected()}
          onChange={() => table.toggleAllPageRowsSelected()}
        />
      </Box>
    ),
    size: 38,
    cell: ({ row }) => (
      <Box
        component="label"
        className={styles.checkboxHitbox}
        htmlFor={`path-table-select-${row.id}`}
        onClick={(e) => e.stopPropagation()}
      >
        <Checkbox
          id={`path-table-select-${row.id}`}
          name={row.original.name}
          title={`Select ${row.original.name}`}
          checked={row.getIsSelected()}
          onClick={(event) => {
            event.stopPropagation();
          }}
          onChange={() => row.toggleSelected()}
        />
      </Box>
    ),
  },
  {
    accessorKey: "type",
    header: "",
    size: 50,
    cell: ({ row }) =>
      row.original.type === PathType.TopLevelAddress ? (
        <Box className={styles.folderIcon}>
          <IconBucket className={styles.icon} size={16} stroke={2.5} />
        </Box>
      ) : row.original.type === PathType.Folder ? (
        <Box className={styles.folderIcon}>
          <IconFolder className={styles.icon} size={16} stroke={2.5} />
        </Box>
      ) : (
        <Box className={styles.fileIcon}>
          <IconFile className={styles.icon} size={16} stroke={2.5} />
        </Box>
      ),
  },
  {
    accessorKey: "resourceAddress",
    header: "Name",
    size: 200,
    cell: ({ row }) => {
      return <span className={styles.cellContent}>{row.original.name}</span>;
    },
  },
  {
    accessorKey: "metadata.dataObjectSize",
    header: "Size",
    size: 100,
    cell: ({ row }) => row.original.type === PathType.File && row.original.size,
  },
  {
    accessorKey: "metadata.lastModificationTimestamp",
    header: "Modified",
    size: 160,
    cell: ({ row }) => row.original.type === PathType.File && row.original.modified,
  },
  {
    accessorKey: "displayedType",
    header: "Type",
    size: 80,
    cell: ({ row }) => row.original.displayedType,
  },
];
