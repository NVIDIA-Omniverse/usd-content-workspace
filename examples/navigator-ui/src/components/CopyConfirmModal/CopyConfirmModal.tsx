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

import { useState } from "react";
import { Box, Button, Checkbox, Group, Stack, Text } from "@mantine/core";
import { ContextModalProps } from "@mantine/modals";
import { Path } from "@/state/path";
import { ClipboardOperation } from "@/state/slices/clipboardSlice";
import { decodePathName } from "@/util/url";

export type CopyConfirmModalProps = ContextModalProps<{
  source: Path[];
  destination: string;
  operation?: ClipboardOperation;
  onConfirm: (destination: string, overwrite: boolean) => void;
}>;

export function CopyConfirmModal({
  id,
  context,
  innerProps: { source, destination, operation = ClipboardOperation.Copy, onConfirm },
}: CopyConfirmModalProps) {
  const [overwrite, setOverwrite] = useState(true);

  function confirm() {
    onConfirm(destination, overwrite);
    context.closeModal(id);
  }

  return (
    <Stack>
      <Text>
        Are you sure you want to {operation === ClipboardOperation.Copy ? "copy" : "cut"}{" "}
        {source.length === 1 ? source[0].name : `${source.length} items`}?
      </Text>

      <Box
        p="sm"
        style={{
          backgroundColor: overwrite ? "var(--mantine-color-red-light-hover)" : "transparent",
          borderRadius: "var(--mantine-radius-sm)",
        }}
      >
        <Checkbox
          label={
            <Text size="sm" ta="center" style={{ wordBreak: "break-all" }}>
              {overwrite ? "Replace" : "Skip"} resources with the same name in {decodePathName(destination)}
            </Text>
          }
          checked={overwrite}
          onChange={(event) => setOverwrite(event.currentTarget.checked)}
          styles={{
            body: { width: "100%", alignItems: "center", gap: "var(--mantine-spacing-sm)" },
            labelWrapper: { width: "100%" },
          }}
        />
      </Box>

      <Group>
        <Button color={operation === ClipboardOperation.Copy ? "green" : "red"} onClick={() => confirm()}>
          Paste
        </Button>
        <Button onClick={() => context.closeModal(id)}>Cancel</Button>
      </Group>
    </Stack>
  );
}
