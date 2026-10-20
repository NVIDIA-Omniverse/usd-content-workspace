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
import { useCallback, useState } from "react";
import { useSearchParams } from "react-router-dom";

export interface CursorPagination {
  cursor: string | undefined;
  hasPrevious: boolean;
  goNext: (nextCursor: string) => void;
  goPrevious: () => void;
  /** Clears the cursor from the URL and resets backward navigation history. */
  reset: () => void;
}

/**
 * Cursor-based pagination backed by URL search params.
 *
 * Forward/backward cursors are stored in the URL so that individual pages are
 * shareable and survive a refresh. Backward navigation relies on an in-memory
 * history stack — if a user opens a URL that already contains a cursor, the
 * "Previous" button will be disabled until they navigate forward at least once.
 */
export function useCursorPagination(paramName = "cursor"): CursorPagination {
  const [searchParams, setSearchParams] = useSearchParams();
  const [history, setHistory] = useState<(string | undefined)[]>([]);

  const cursor = searchParams.get(paramName) ?? undefined;

  const goNext = useCallback(
    (nextCursor: string) => {
      setHistory((prev) => [...prev, cursor]);
      setSearchParams((prev) => {
        const next = new URLSearchParams(prev);
        next.set(paramName, nextCursor);
        return next;
      });
    },
    [cursor, paramName, setSearchParams],
  );

  const goPrevious = useCallback(() => {
    setHistory((prev) => {
      const next = [...prev];
      const previous = next.pop();
      setSearchParams((p) => {
        const params = new URLSearchParams(p);
        if (previous) {
          params.set(paramName, previous);
        } else {
          params.delete(paramName);
        }
        return params;
      });
      return next;
    });
  }, [paramName, setSearchParams]);

  const reset = useCallback(() => {
    setHistory([]);
    setSearchParams((prev) => {
      const next = new URLSearchParams(prev);
      next.delete(paramName);
      return next;
    });
  }, [paramName, setSearchParams]);

  return {
    cursor,
    hasPrevious: history.length > 0,
    goNext,
    goPrevious,
    reset,
  };
}
