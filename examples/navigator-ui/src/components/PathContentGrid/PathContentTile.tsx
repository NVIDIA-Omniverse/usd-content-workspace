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
import { useNavigate } from "react-router-dom";
import { Checkbox, ElementProps, Image, Stack, StackProps, Text } from "@mantine/core";
import { constructPathNavLink } from "@/hooks/usePathNavigation";
import { DEFAULT_IMAGES, usePathThumbnail } from "@/hooks/usePathThumbnail";
import { Connection, useAppStore } from "@/state/appStore";
import { Path } from "@/state/path";
import { PathContentTileIcon } from "./PathContentTileIcon";
import styles from "./PathContentTile.module.css";

export interface PathContentTileProps extends StackProps, ElementProps<"div"> {
  connection: Connection;
  height: number;
  padding: number;
  path: Path;
  selected?: boolean;
  thumbnailSize: number;
  width: number;

  onContextMenu?: (event: React.MouseEvent<HTMLDivElement> & PathContentTileEventData) => void;
  onSelect?: (event: React.MouseEvent<HTMLDivElement> & PathContentTileEventData) => void;
}

export interface PathContentTileEventData {
  data: {
    path: Path;
  };
}

export const PathContentTile = memo(
  ({
    connection,
    height,
    padding,
    path,
    selected = false,
    thumbnailSize,
    width,
    onSelect,
    ...props
  }: PathContentTileProps) => {
    const setSelectedPaths = useAppStore((state) => state.setSelectedPaths);
    const toggleSelectedPath = useAppStore((state) => state.toggleSelectedPath);
    const imageSrc = usePathThumbnail(path, connection);

    const hasThumbnail = imageSrc !== DEFAULT_IMAGES[path.type];
    const handleClick = (event: React.MouseEvent<HTMLDivElement>) => {
      event.stopPropagation();
      Object.defineProperty(event, "data", {
        value: { path },
      });
      const eventWithData = event as React.MouseEvent<HTMLDivElement> & PathContentTileEventData;
      props.onClick?.(event);
      onSelect?.(eventWithData);
      if (!event.defaultPrevented) {
        setSelectedPaths(connection.endpoint, [path]);
      }
    };

    const navigate = useNavigate();
    const handleDoubleClick = (event: React.MouseEvent<HTMLDivElement>) => {
      props.onDoubleClick?.(event);
      if (!event.defaultPrevented && path.isFolderLike) {
        void navigate(constructPathNavLink(connection.endpoint, path.resourceAddress));
      }
    };

    const handleContextMenu = (event: React.MouseEvent<HTMLDivElement>) => {
      Object.defineProperty(event, "data", {
        value: {
          path,
        },
      });
      props.onContextMenu?.(event as React.MouseEvent<HTMLDivElement> & PathContentTileEventData);
    };

    const handleSelectionChange = (event: React.MouseEvent) => {
      event.preventDefault();
      event.stopPropagation();
      toggleSelectedPath(connection.endpoint, path);
    };

    return (
      <Stack
        className={styles.tile}
        align="center"
        aria-label={path.name}
        aria-selected={selected}
        role="gridcell"
        w={width}
        h={height}
        p={padding}
        gap={padding}
        {...props}
        onClick={handleClick}
        onContextMenu={handleContextMenu}
        onDoubleClick={handleDoubleClick}
      >
        <Checkbox
          className={styles.tileCheckbox}
          checked={selected}
          title={selected ? `Deselect ${path.name}` : `Select ${path.name}`}
          onClick={stopPropagation}
          onMouseDown={handleSelectionChange}
          onChange={stopPropagation}
        />

        <div
          className={styles.thumbnailWrapper}
          style={{ width: thumbnailSize, height: thumbnailSize, flex: `0 0 ${thumbnailSize}px` }}
        >
          <Image src={imageSrc} w={thumbnailSize} fit="cover" />
          <PathContentTileIcon hasThumbnail={hasThumbnail} type={path.type} />
        </div>
        <Text
          className={styles.tileText}
          title={path.name}
          truncate="end"
          w={width}
          p={padding}
          h={height - thumbnailSize - padding * 2}
        >
          {path.name}
        </Text>
      </Stack>
    );
  }
);
PathContentTile.displayName = "PathContentTile";

function stopPropagation(event: React.SyntheticEvent) {
  event.stopPropagation();
  event.preventDefault();
}
