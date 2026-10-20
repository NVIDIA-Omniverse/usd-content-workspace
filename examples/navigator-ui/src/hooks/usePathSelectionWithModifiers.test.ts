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

import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { Path } from "@/state/path";
import { createMockFile } from "@/test-utils/mockPath";
import { usePathSelectionWithModifiers } from "./usePathSelectionWithModifiers";

function createMouseEvent(overrides: { shiftKey?: boolean; ctrlKey?: boolean; metaKey?: boolean } = {}) {
  return {
    shiftKey: false,
    ctrlKey: false,
    metaKey: false,
    preventDefault: vi.fn(),
    ...overrides,
  } as unknown as React.MouseEvent;
}

describe("usePathSelectionWithModifiers", () => {
  const pathA = createMockFile("a");
  const pathB = createMockFile("b");
  const pathC = createMockFile("c");
  const data: Path[] = [pathA, pathB, pathC];

  it("single click selects only the clicked path", () => {
    const onSelectionChange = vi.fn();
    const { result } = renderHook(() => usePathSelectionWithModifiers(data, new Set(), onSelectionChange));

    act(() => {
      result.current(pathB, createMouseEvent());
    });

    expect(onSelectionChange).toHaveBeenCalledTimes(1);
    expect(onSelectionChange).toHaveBeenCalledWith([pathB]);
  });

  it("shift-click selects range from last clicked index to current", () => {
    const onSelectionChange = vi.fn();
    const { result } = renderHook(() => usePathSelectionWithModifiers(data, new Set(), onSelectionChange));

    act(() => {
      result.current(pathA, createMouseEvent());
    });
    expect(onSelectionChange).toHaveBeenCalledWith([pathA]);
    onSelectionChange.mockClear();

    act(() => {
      result.current(pathC, createMouseEvent({ shiftKey: true }));
    });
    expect(onSelectionChange).toHaveBeenCalledTimes(1);
    expect(onSelectionChange).toHaveBeenCalledWith([pathA, pathB, pathC]);
  });

  it("shift-click selects range when clicking backwards", () => {
    const onSelectionChange = vi.fn();
    const { result } = renderHook(() => usePathSelectionWithModifiers(data, new Set(), onSelectionChange));

    act(() => {
      result.current(pathC, createMouseEvent());
    });
    onSelectionChange.mockClear();

    act(() => {
      result.current(pathA, createMouseEvent({ shiftKey: true }));
    });
    expect(onSelectionChange).toHaveBeenCalledWith([pathA, pathB, pathC]);
  });

  it("shift-click without prior click does single selection", () => {
    const onSelectionChange = vi.fn();
    const { result } = renderHook(() => usePathSelectionWithModifiers(data, new Set(), onSelectionChange));

    act(() => {
      result.current(pathB, createMouseEvent({ shiftKey: true }));
    });

    expect(onSelectionChange).toHaveBeenCalledWith([pathB]);
  });

  it("ctrl-click adds path to selection", () => {
    const onSelectionChange = vi.fn();
    const { result } = renderHook(() => usePathSelectionWithModifiers(data, new Set([pathA]), onSelectionChange));

    act(() => {
      result.current(pathB, createMouseEvent({ ctrlKey: true }));
    });

    expect(onSelectionChange).toHaveBeenCalledWith([pathA, pathB]);
  });

  it("ctrl-click removes path from selection when already selected", () => {
    const onSelectionChange = vi.fn();
    const { result } = renderHook(() =>
      usePathSelectionWithModifiers(data, new Set([pathA, pathB]), onSelectionChange)
    );

    act(() => {
      result.current(pathB, createMouseEvent({ ctrlKey: true }));
    });

    expect(onSelectionChange).toHaveBeenCalledWith([pathA]);
  });

  it("meta-key (Cmd) click toggles selection like ctrl", () => {
    const onSelectionChange = vi.fn();
    const { result } = renderHook(() => usePathSelectionWithModifiers(data, new Set([pathA]), onSelectionChange));

    act(() => {
      result.current(pathC, createMouseEvent({ metaKey: true }));
    });

    expect(onSelectionChange).toHaveBeenCalledWith([pathA, pathC]);
  });

  it("resets the shift-click anchor when current selection becomes empty", () => {
    let currentSelection = new Set<Path>();
    const onSelectionChange = vi.fn((paths: Path[]) => {
      currentSelection = new Set(paths);
    });
    const { rerender, result } = renderHook(() =>
      usePathSelectionWithModifiers(data, currentSelection, onSelectionChange)
    );

    act(() => {
      result.current(pathA, createMouseEvent());
    });

    act(() => {
      currentSelection = new Set();
      rerender();
    });
    onSelectionChange.mockClear();

    act(() => {
      result.current(pathC, createMouseEvent({ shiftKey: true }));
    });

    expect(onSelectionChange).toHaveBeenCalledWith([pathC]);
  });

  it("does nothing when path is not in data", () => {
    const onSelectionChange = vi.fn();
    const otherPath = createMockFile("other");
    const { result } = renderHook(() => usePathSelectionWithModifiers(data, new Set(), onSelectionChange));

    act(() => {
      result.current(otherPath, createMouseEvent());
    });

    expect(onSelectionChange).not.toHaveBeenCalled();
  });
});
