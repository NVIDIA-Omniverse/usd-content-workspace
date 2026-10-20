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

export function createMockFileHandle(name: string): FileSystemFileHandle {
  const blobParts: BlobPart[] = [];
  return {
    name,
    kind: "file",
    getFile: vi.fn().mockImplementation(() => {
      return Promise.resolve(new File(blobParts, name));
    }),
    createWritable: vi.fn().mockResolvedValue(
      new WritableStream({
        write(chunk: BlobPart) {
          blobParts.push(chunk);
          return Promise.resolve();
        },
        close() {
          return Promise.resolve();
        },
      }) as unknown as FileSystemWritableFileStream
    ),
  } as unknown as FileSystemFileHandle;
}

export function createMockDirectoryHandle(
  name: string,
  entries: (FileSystemFileHandle | FileSystemDirectoryHandle)[]
): FileSystemDirectoryHandle {
  return {
    name,
    kind: "directory",
    values: vi.fn().mockImplementation(function* () {
      for (const entry of entries) {
        yield entry;
      }
    }),
    getDirectoryHandle: vi.fn().mockImplementation((name: string) => {
      const entry = createMockDirectoryHandle(name, []);
      entries.push(entry);
      return entry;
    }),
    getFileHandle: vi.fn().mockImplementation((name: string) => {
      const entry = createMockFileHandle(name);
      entries.push(entry);
      return entry;
    }),
    removeEntry: vi.fn().mockImplementation((name: string) => {
      const index = entries.findIndex((e) => e.name === name);
      if (index !== -1) {
        entries.splice(index, 1);
      }
      return Promise.resolve();
    }),
  } as unknown as FileSystemDirectoryHandle;
}
