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
import { Path } from "@/state/path";
import { ClipboardOperation } from "@/state/slices/clipboardSlice";
import { screen } from "@/test-utils";
import { createMockFile, createMockFolder } from "@/test-utils/mockPath";
import { renderModal } from "@/test-utils/render";

describe("CopyConfirmModal", () => {
  const onConfirm = vi.fn();
  const onClose = vi.fn();

  const singleResource: Path[] = [createMockFile("test-file.txt")];
  const multipleResources: Path[] = [createMockFile("test-file.txt"), createMockFolder("test-folder")];

  const destination = "/Projects/Test%20Folder/";
  const decodedDestination = "/Projects/Test Folder/";
  const replaceLabel = `Replace resources with the same name in ${decodedDestination}`;
  const skipLabel = `Skip resources with the same name in ${decodedDestination}`;

  beforeEach(() => {
    onConfirm.mockClear();
    onClose.mockClear();
  });

  it("renders copy description for a single resource", async () => {
    await renderModal({
      modal: "copy",
      innerProps: {
        source: singleResource,
        destination,
        operation: ClipboardOperation.Copy,
        onConfirm,
      },
      onClose,
    });

    expect(screen.getByText(`Are you sure you want to copy ${singleResource[0].name}?`)).toBeVisible();
  });

  it("renders copy description for multiple resources", async () => {
    await renderModal({
      modal: "copy",
      innerProps: {
        source: multipleResources,
        destination,
        operation: ClipboardOperation.Copy,
        onConfirm,
      },
      onClose,
    });
    expect(screen.getByText(`Are you sure you want to copy ${multipleResources.length} items?`)).toBeVisible();
  });

  it("confirms the copy", async () => {
    const user = userEvent.setup();

    await renderModal({
      modal: "copy",
      innerProps: {
        source: singleResource,
        destination,
        operation: ClipboardOperation.Copy,
        onConfirm,
      },
      onClose,
    });

    await user.click(screen.getByRole("button", { name: "Paste" }));

    expect(onConfirm).toHaveBeenCalledWith(destination, true);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("shows the overwrite checkbox, checked by default, for a copy operation", async () => {
    await renderModal({
      modal: "copy",
      innerProps: {
        source: singleResource,
        destination,
        operation: ClipboardOperation.Copy,
        onConfirm,
      },
      onClose,
    });

    expect(screen.getByRole("checkbox", { name: replaceLabel })).toBeChecked();
  });

  it("confirms the copy with overwrite disabled when the checkbox is unchecked", async () => {
    const user = userEvent.setup();

    await renderModal({
      modal: "copy",
      innerProps: {
        source: singleResource,
        destination,
        operation: ClipboardOperation.Copy,
        onConfirm,
      },
      onClose,
    });

    await user.click(screen.getByRole("checkbox", { name: replaceLabel }));
    await user.click(screen.getByRole("button", { name: "Paste" }));

    expect(onConfirm).toHaveBeenCalledWith(destination, false);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("changes the label to Skip when the checkbox is unchecked", async () => {
    const user = userEvent.setup();

    await renderModal({
      modal: "copy",
      innerProps: {
        source: singleResource,
        destination,
        operation: ClipboardOperation.Copy,
        onConfirm,
      },
      onClose,
    });

    await user.click(screen.getByRole("checkbox", { name: replaceLabel }));

    expect(screen.getByRole("checkbox", { name: skipLabel })).not.toBeChecked();
  });

  it("shows the overwrite checkbox, checked by default, for a cut operation too", async () => {
    await renderModal({
      modal: "copy",
      innerProps: {
        source: singleResource,
        destination,
        operation: ClipboardOperation.Cut,
        onConfirm,
      },
      onClose,
    });

    expect(screen.getByRole("checkbox", { name: replaceLabel })).toBeChecked();
  });

  it("confirms the cut with overwrite disabled when the checkbox is unchecked", async () => {
    const user = userEvent.setup();

    await renderModal({
      modal: "copy",
      innerProps: {
        source: singleResource,
        destination,
        operation: ClipboardOperation.Cut,
        onConfirm,
      },
      onClose,
    });

    await user.click(screen.getByRole("checkbox", { name: replaceLabel }));
    await user.click(screen.getByRole("button", { name: "Paste" }));

    expect(onConfirm).toHaveBeenCalledWith(destination, false);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("renders move description for a cut operation", async () => {
    await renderModal({
      modal: "copy",
      innerProps: {
        source: singleResource,
        destination,
        operation: ClipboardOperation.Cut,
        onConfirm,
      },
      onClose,
    });

    expect(screen.getByText(`Are you sure you want to cut ${singleResource[0].name}?`)).toBeVisible();
  });

  it("confirms the cut", async () => {
    const user = userEvent.setup();

    await renderModal({
      modal: "copy",
      innerProps: {
        source: singleResource,
        destination,
        operation: ClipboardOperation.Cut,
        onConfirm,
      },
      onClose,
    });

    await user.click(screen.getByRole("button", { name: "Paste" }));

    expect(onConfirm).toHaveBeenCalledWith(destination, true);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("cancels the copy", async () => {
    const user = userEvent.setup();

    await renderModal({
      modal: "copy",
      innerProps: {
        source: singleResource,
        destination,
        operation: ClipboardOperation.Copy,
        onConfirm,
      },
      onClose,
    });

    await user.click(screen.getByRole("button", { name: "Cancel" }));

    expect(onConfirm).not.toHaveBeenCalled();
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
