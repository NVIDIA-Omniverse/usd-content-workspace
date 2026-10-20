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

import { ReactNode } from "react";
import { QueryClientProvider } from "@tanstack/react-query";
import { act, render as testingLibraryRender } from "@testing-library/react";
import { ContextMenuProvider } from "mantine-contextmenu";
import { MantineProvider, v8CssVariablesResolver } from "@mantine/core";
import { modals } from "@mantine/modals";
import { MantineModal, MantineModals, ModalSettings, OpenContextModal } from "@mantine/modals/lib/context";
import { ModalsProvider } from "@/providers/ModalsProvider";
import { createTestQueryClient } from "@/test-utils/utils";
import { theme } from "@/theme";

export interface RenderOptions {
  modalProps?: ModalSettings;
}

export function render(ui: ReactNode, options?: RenderOptions) {
  return testingLibraryRender(ui, {
    wrapper: ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={createTestQueryClient()}>
        <MantineProvider theme={theme} forceColorScheme="dark" cssVariablesResolver={v8CssVariablesResolver} env="test">
          <ContextMenuProvider>
            <ModalsProvider modalProps={options?.modalProps}>{children}</ModalsProvider>
          </ContextMenuProvider>
        </MantineProvider>
      </QueryClientProvider>
    ),
  });
}

export async function renderModal<TKey extends MantineModal>(
  modal: TestModalProps<TKey>
): Promise<ReturnType<typeof modals.openContextModal>> {
  render(<div />); // Rendering a modal only requires ModalsProvider.
  return act(() => {
    return modals.openContextModal({ ...modal, withinPortal: false });
  });
}

export type TestModalProps<TKey extends MantineModal> = OpenContextModal<
  Parameters<MantineModals[TKey]>[0]["innerProps"]
> & {
  modal: TKey;
};
