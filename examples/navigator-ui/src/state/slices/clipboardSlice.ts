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

import { Path } from "@/state/path";
import { Slice } from "@/state/slices/types";

export interface ClipboardSlice {
  clipboard: {
    sourcePath: string;
    items: Map<string, Path[]>;
  };
  clipboardOperation: ClipboardOperation;

  copyToClipboard: (endpoint: string, sourcePath: string, resource: Path[], operation?: ClipboardOperation) => void;
  clearClipboard: (endpoint: string) => void;
}

export const enum ClipboardOperation {
  Copy = "Copy",
  Cut = "Cut",
}

export const createClipboardSlice: Slice<ClipboardSlice> = (set) => ({
  clipboard: {
    sourcePath: "",
    items: new Map(),
  },
  clipboardOperation: ClipboardOperation.Copy,

  copyToClipboard: (
    endpoint: string,
    sourcePath: string,
    resource: Path[],
    operation: ClipboardOperation = ClipboardOperation.Copy
  ) => {
    set((state) => {
      state.clipboardOperation = operation;
      state.clipboard.items.set(endpoint, resource);
      state.clipboard.sourcePath = sourcePath;
      return state;
    });
  },
  clearClipboard: (endpoint: string) => {
    set((state) => {
      state.clipboardOperation = ClipboardOperation.Copy;
      state.clipboard.items.set(endpoint, []);
      state.clipboard.sourcePath = "";
      return state;
    });
  },
});
