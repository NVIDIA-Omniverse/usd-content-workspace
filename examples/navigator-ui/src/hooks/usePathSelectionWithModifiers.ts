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

import { useCallback, useEffect, useMemo, useRef } from "react";
import { Path } from "@/state/path";

/**
 * Returns a click handler that implements shift-click (range selection) and
 * ctrl/cmd-click (toggle selection) for a list of paths. Call preventDefault()
 * on the event in the caller if the default single-selection behavior should
 * be skipped (e.g. when the handler fully controls selection).
 */
export function usePathSelectionWithModifiers(
  data: Path[],
  currentSelection: Set<Path> | undefined,
  onSelectionChange: (paths: Path[]) => void
): (path: Path, event: React.MouseEvent) => void {
  /** Range selection anchor: set on plain click, used as start of range while shift is held. */
  const rangeAnchorPathIdRef = useRef<string | null>(null);
  const pathToIndex = useMemo(() => new Map(data.map((path, i) => [path.resourceAddress, i])), [data]);
  const getIndex = useCallback(
    (resourceAddress: string): number => pathToIndex.get(resourceAddress) ?? -1,
    [pathToIndex]
  );

  useEffect(() => {
    if (!currentSelection || currentSelection.size === 0) {
      rangeAnchorPathIdRef.current = null;
    }
  }, [currentSelection]);

  return useCallback(
    (path: Path, event: React.MouseEvent) => {
      const currentIndex = getIndex(path.resourceAddress);
      if (currentIndex < 0) {
        return;
      }

      if (event.shiftKey && rangeAnchorPathIdRef.current !== null) {
        const startIndex = getIndex(rangeAnchorPathIdRef.current);
        if (startIndex >= 0) {
          const start = startIndex;
          const end = currentIndex;
          const [from, to] = start <= end ? [start, end] : [end, start];
          const rangePaths = data.slice(from, to + 1);
          onSelectionChange(rangePaths);
        } else {
          onSelectionChange([path]);
        }
      } else if (event.ctrlKey || event.metaKey) {
        const newSelection = new Set(currentSelection);
        if (newSelection.has(path)) {
          newSelection.delete(path);
        } else {
          newSelection.add(path);
        }
        onSelectionChange(Array.from(newSelection));
      } else {
        onSelectionChange([path]);
        rangeAnchorPathIdRef.current = path.resourceAddress;
      }
    },
    [data, currentSelection, onSelectionChange, getIndex]
  );
}
