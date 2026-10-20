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
import { IconPlug } from "@tabler/icons-react";
import { modals } from "@mantine/modals";
import { PathTreeNode, PathTreeNodeProps } from "@/components/PathTree/PathTreeNode";
import { useAddConnection } from "@/hooks/useAddConnection";
import { useServiceApi } from "@/hooks/useServiceApi";
import { isHttpError } from "@/services/api";
import { authorize, getAuthConfig } from "@/services/auth";

export function PathTreeAddConnectionNode({ node, item }: PathTreeNodeProps) {
  const addConnection = useAddConnection();
  const { getApi } = useServiceApi();

  const onAddConnection = async (hostname: string) => {
    try {
      // Check if deployment can be accessed without authentication
      const storage = await getApi(hostname, "filefolder");

      // If storage is available, add the discovered storage endpoint
      const auth = null;
      addConnection(hostname, storage ? [storage.baseUrl] : [], auth);

      modals.closeAll();
    } catch (error) {
      if (isHttpError(error) && error.response.status === 401) {
        // This deployment requires authentication,
        // get authentication information and redirect the user to the IdP
        const config = await getAuthConfig(hostname);
        if (config) {
          await authorize(
            {
              urls: {
                discovery: hostname,
              },
            },
            config
          );
          return;
        }

        throw new Error(`${hostname} requires authentication but authentication configuration is not available.`);
      }
      throw error;
    }
  };

  const openConnectionModal = (event: React.MouseEvent) => {
    event.preventDefault();

    modals.openContextModal({
      modal: "connect",
      title: "Connect",
      centered: true,
      innerProps: {
        onAddConnection,
      },
    });
  };

  return (
    <PathTreeNode
      node={node}
      item={item}
      icon={<IconPlug />}
      expandable={false}
      title="Add Connection"
      onClick={openConnectionModal}
    >
      Add Connection
    </PathTreeNode>
  );
}
