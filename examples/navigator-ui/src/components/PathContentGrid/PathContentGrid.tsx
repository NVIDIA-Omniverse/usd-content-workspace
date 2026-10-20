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

import React, { RefObject, useCallback, useEffect, useMemo, useRef } from "react";
import { useVirtualizer, Virtualizer } from "@tanstack/react-virtual";
import { Box, Group } from "@mantine/core";
import { useElementSize, useMergedRef } from "@mantine/hooks";
import { VirtualContainer } from "@/components/VirtualContainer";
import { usePathContextMenu } from "@/hooks/usePathContextMenu";
import { usePathSelectionWithModifiers } from "@/hooks/usePathSelectionWithModifiers";
import { useAppStore } from "@/state/appStore";
import { Path } from "@/state/path";
import { Connection } from "@/state/slices/connectionSlice";
import { UploadDropzone } from "../UploadDropzone/UploadDropzone";
import { PathContentTile, PathContentTileEventData } from "./PathContentTile";
import styles from "./PathContentGrid.module.css";

export interface PathContentGridProps {
  connection: Connection;
  data: Path[];
  thumbnailSize?: number;
  tileWidth?: number;
  tileHeight?: number;
  tilePadding?: number;
  tileGap?: number;
}

export function PathContentGrid({
  connection,
  data,
  thumbnailSize = 128,
  tileWidth = 150,
  tileHeight = 172,
  tilePadding = 5,
  tileGap = 10,
}: PathContentGridProps) {
  const { ref: sizeRef, width } = useElementSize<HTMLDivElement>();
  const scrollRef = useRef<HTMLDivElement>(null);
  const containerRef = useMergedRef(sizeRef, scrollRef);

  const rows = useMemo((): Path[][] => {
    if (!width) {
      // The width is not calculated for the container yet
      return [];
    }

    const rowWidth = width - tileGap * 2;
    let cellCount = Math.floor(rowWidth / (tileWidth + tileGap));
    if (cellCount <= 0) {
      // Container is too small, only one cell can be displayed on a row
      cellCount = 1;
    }

    const rows: Path[][] = [];
    for (let i = 0; i < data.length; i += cellCount) {
      rows.push(data.slice(i, i + cellCount));
    }

    return rows;
  }, [data, width, tileGap, tileWidth]);

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => tileHeight + tileGap,
    overscan: 3,
  });

  const selectedPaths = useAppStore((state) => state.connections[connection.endpoint]?.selectedPaths) ?? emptySelection;
  const selectedPathsRef = useRef<Set<Path>>(selectedPaths);
  selectedPathsRef.current = selectedPaths;

  const setSelectedPaths = useAppStore((state) => state.setSelectedPaths);
  useGridScrollRestoration(virtualizer, selectedPathsRef, rows);

  const handlePathClick = usePathSelectionWithModifiers(
    data,
    selectedPaths,
    useCallback(
      (paths: Path[]) => {
        setSelectedPaths(connection.endpoint, paths);
      },
      [connection.endpoint, setSelectedPaths]
    )
  );
  const handleTileClick = useCallback(
    (event: React.MouseEvent<HTMLDivElement> & PathContentTileEventData) => {
      handlePathClick(event.data.path, event);
      event.preventDefault();
    },
    [handlePathClick]
  );

  const contextMenu = usePathContextMenu(connection);
  const handleContextMenu = useCallback(
    (event: React.MouseEvent<HTMLDivElement> & PathContentTileEventData) => {
      if (selectedPathsRef.current.has(event.data.path)) {
        void contextMenu(Array.from(selectedPathsRef.current))(event);
        return;
      }
      setSelectedPaths(connection.endpoint, [event.data.path]);
      void contextMenu([event.data.path])(event);
    },
    [connection.endpoint, setSelectedPaths, contextMenu]
  );

  const handleEmptyContentClick = useCallback(() => {
    if (selectedPathsRef.current.size > 0) {
      setSelectedPaths(connection.endpoint, []);
    }
  }, [connection.endpoint, setSelectedPaths]);

  return (
    <VirtualContainer role="grid" containerRef={containerRef} className={styles.grid} onClick={handleEmptyContentClick}>
      <UploadDropzone connection={connection}>
        <Box pos="relative" h={virtualizer.getTotalSize()}>
          {virtualizer.getVirtualItems().map((item) => {
            const row = rows[item.index];
            return (
              <Group
                key={item.index}
                role="row"
                pos="absolute"
                m={tileGap}
                gap={tileGap}
                style={{ top: `${item.start}px` }}
              >
                {row.map((path) => (
                  <PathContentTile
                    key={path.resourceAddress}
                    connection={connection}
                    height={tileHeight}
                    padding={tilePadding}
                    path={path}
                    selected={selectedPaths.has(path)}
                    thumbnailSize={thumbnailSize}
                    width={tileWidth}
                    onContextMenu={handleContextMenu}
                    onSelect={handleTileClick}
                  />
                ))}
              </Group>
            );
          })}
        </Box>
      </UploadDropzone>
    </VirtualContainer>
  );
}

function useGridScrollRestoration(
  virtualizer: Virtualizer<HTMLDivElement, Element>,
  selectedPathsRef: RefObject<Set<Path>>,
  rows: Path[][]
) {
  const detailsVisible = useAppStore((state) => state.detailsPanel);

  const virtualizerRef = useRef(virtualizer);
  virtualizerRef.current = virtualizer;

  const pathToRowIndex = useMemo(
    () =>
      new Map(rows.flatMap((row, rowIndex) => row.map((path) => [path.resourceAddress, rowIndex] as [string, number]))),
    [rows]
  );
  const pathToRowIndexRef = useRef(pathToRowIndex);
  pathToRowIndexRef.current = pathToRowIndex;

  useEffect(() => {
    const timer = setTimeout(() => {
      const selectedPaths = selectedPathsRef.current;
      if (!selectedPaths) {
        return;
      }

      const selectedPath = Array.from(selectedPaths)[0];
      if (!selectedPath) {
        return;
      }
      const selectedRowIndex = pathToRowIndexRef.current?.get(selectedPath.resourceAddress) ?? -1;
      if (selectedRowIndex >= 0) {
        virtualizerRef.current.scrollToIndex(selectedRowIndex, { behavior: "smooth", align: "center" });
      }
    }, 250);
    return () => clearTimeout(timer);
  }, [detailsVisible, selectedPathsRef]);
}

const emptySelection = new Set<Path>();
