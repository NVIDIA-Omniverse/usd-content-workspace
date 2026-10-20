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

import { Button, Group, Stack, Text } from "@mantine/core";
import { ContextModalProps } from "@mantine/modals";
import { Path } from "@/state/path";

export type DeleteConfirmModalProps = ContextModalProps<{
  paths: Path[];
  onConfirm: (resources: Path[]) => void;
}>;

export function DeleteConfirmModal({ id, context, innerProps: { paths, onConfirm } }: DeleteConfirmModalProps) {
  function confirm() {
    onConfirm(paths);
    context.closeModal(id);
  }

  return (
    <Stack>
      <Text>Are you sure you want to delete {paths.length === 1 ? paths[0].name : `${paths.length} items`}?</Text>

      <Group>
        <Button color="red" onClick={() => confirm()}>
          Delete
        </Button>
        <Button onClick={() => context.closeModal(id)}>Cancel</Button>
      </Group>
    </Stack>
  );
}
