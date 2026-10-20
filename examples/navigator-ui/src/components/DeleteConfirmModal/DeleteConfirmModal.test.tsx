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

import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { Path, PathType } from "@/state/path";
import { screen } from "@/test-utils";
import { renderModal } from "@/test-utils/render";

describe("DeleteConfirmModal", () => {
  const onConfirm = vi.fn();
  const onClose = vi.fn();

  const singleResource: Path[] = [
    {
      name: "test-file.txt",
      type: PathType.File,
      displayedType: "txt",
      isFolderLike: false,
      size: "1.00 KB",
      bytes: 1024,
      modified: "02/27/2024 12:00:00",
      resourceAddress: "test-file.txt",
      resourceIdentity: "1",
      metadata: {
        dataObjectSize: 1024,
        lastModifiedTimestamp: new Date().toISOString(),
      },
    },
  ];

  const multipleResources: Path[] = [
    {
      name: "test-file.txt",
      type: PathType.File,
      displayedType: "txt",
      isFolderLike: false,
      size: "1.00 KB",
      bytes: 1024,
      modified: "02/27/2024 12:00:00",
      resourceAddress: "test-file.txt",
      resourceIdentity: "1",
      metadata: {
        dataObjectSize: 1024,
        lastModifiedTimestamp: new Date().toISOString(),
      },
    },
    {
      name: "test-folder",
      type: PathType.Folder,
      displayedType: "Folder",
      isFolderLike: true,
      resourceAddress: "test-folder",
    },
  ];

  beforeEach(() => {
    onConfirm.mockClear();
    onClose.mockClear();
  });

  it("renders description for a single resource", async () => {
    await renderModal({
      modal: "delete",
      innerProps: {
        paths: singleResource,
        onConfirm,
      },
      onClose,
    });

    expect(screen.getByText(`Are you sure you want to delete ${singleResource[0].name}?`)).toBeInTheDocument();
  });

  it("renders description for multiple resources", async () => {
    await renderModal({
      modal: "delete",
      innerProps: {
        paths: multipleResources,
        onConfirm,
      },
      onClose,
    });
    expect(screen.getByText(`Are you sure you want to delete ${multipleResources.length} items?`)).toBeInTheDocument();
  });

  it("confirms the deletion", async () => {
    const user = userEvent.setup();

    await renderModal({
      modal: "delete",
      innerProps: {
        paths: singleResource,
        onConfirm,
      },
      onClose,
    });

    await user.click(screen.getByRole("button", { name: "Delete" }));

    expect(onConfirm).toHaveBeenCalledWith(singleResource);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("cancels the deletion", async () => {
    const user = userEvent.setup();

    await renderModal({
      modal: "delete",
      innerProps: {
        paths: singleResource,
        onConfirm,
      },
      onClose,
    });

    await user.click(screen.getByRole("button", { name: "Cancel" }));

    expect(onConfirm).not.toHaveBeenCalled();
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
