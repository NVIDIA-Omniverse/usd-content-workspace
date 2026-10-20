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

import { Anchor, Button, Image, Stack, Text } from "@mantine/core";
import { ContextModalProps, modals } from "@mantine/modals";
import logo from "@/assets/logo.png";

export type AboutModalProps = ContextModalProps<Record<string, never>>;

export function AboutModal({ id }: AboutModalProps) {
  function close() {
    modals.close(id);
  }

  return (
    <Stack align="center" gap={0} py="md">
      <Image src={logo} w={64} h={64} />
      <Text fw={700} fz="lg">
        Storage Navigator
      </Text>
      <Text c="dimmed" fz="sm">
        {__APP_VERSION__}
      </Text>
      <Anchor href="/ATTRIBUTIONS.md" target="_blank" fz="sm" mt="lg">
        Third-party licenses
      </Anchor>
      <Button color="gray" variant="default" mt="sm" onClick={close}>
        Close
      </Button>
    </Stack>
  );
}
