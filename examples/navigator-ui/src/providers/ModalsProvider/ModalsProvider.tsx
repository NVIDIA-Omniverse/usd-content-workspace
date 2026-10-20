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

import React from "react";
import { ModalsProvider as MantineModalsProvider } from "@mantine/modals";
import { ModalSettings } from "@mantine/modals/lib/context";
import { AboutModal } from "@/components/AboutModal";
import { ConnectModal } from "@/components/ConnectModal";
import { CopyConfirmModal } from "@/components/CopyConfirmModal";
import { DeleteConfirmModal } from "@/components/DeleteConfirmModal";
import { NewFolderModal } from "@/components/NewFolderModal";
import { RenameModal } from "@/components/RenameModal/RenameModal";

export interface ModalsProviderProps {
  children?: React.ReactNode;
  modalProps?: ModalSettings;
}

export function ModalsProvider({ children, modalProps }: ModalsProviderProps) {
  return (
    <MantineModalsProvider modals={modals} modalProps={modalProps}>
      {children}
    </MantineModalsProvider>
  );
}

const modals = {
  about: AboutModal,
  newFolder: NewFolderModal,
  rename: RenameModal,
  copy: CopyConfirmModal,
  delete: DeleteConfirmModal,
  connect: ConnectModal,
};

declare module "@mantine/modals" {
  export interface MantineModalsOverride {
    modals: typeof modals;
  }
}
