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

import { Attributes } from "@/components/Attributes";
import { DetailsPanel } from "@/components/DetailsPanel";
import { DetailsPanelTabs } from "@/components/DetailsPanelTabs";
import { PathPermissions } from "@/components/PathPermissions";
import { PathTags } from "@/components/PathTags";
import { PathText } from "@/components/PathText";
import { usePathMetadata } from "@/hooks/usePathMetadata";
import { useAppStore } from "@/state/appStore";
import { PathType } from "@/state/path";
import { DetailsPanelTab } from "@/state/slices/applicationSlice";
import { Connection } from "@/state/slices/connectionSlice";
import { getPathName } from "@/util/url";

export interface PathCurrentFolderDetailsProps {
  connection: Connection;
}

export function PathCurrentFolderDetails({ connection }: PathCurrentFolderDetailsProps) {
  const resourceAddress = connection.currentFolder || connection.endpoint;
  const isFolder = resourceAddress !== connection.endpoint;
  const metadata = usePathMetadata(resourceAddress, connection, isFolder);
  const resolvedAccess = metadata.data?.acl.value.toSorted().join(", ");

  const detailsPanelTab = useAppStore((state) => state.detailsPanelTab);
  const setDetailsPanelTab = useAppStore((state) => state.setDetailsPanelTab);

  return (
    <DetailsPanel>
      <DetailsPanel.Title>{getPathName(resourceAddress)}</DetailsPanel.Title>
      <PathText bg="dark" px="md" pb="xs" title={resourceAddress}>
        {resourceAddress}
      </PathText>

      {resourceAddress !== connection.endpoint && (
        <DetailsPanelTabs
          value={detailsPanelTab}
          onChange={(value) => value && setDetailsPanelTab(value as DetailsPanelTab)}
        >
          <DetailsPanelTabs.List>
            <DetailsPanelTabs.Tab value="details">Details</DetailsPanelTabs.Tab>
            <DetailsPanelTabs.Tab value="permissions">Permissions</DetailsPanelTabs.Tab>
            <DetailsPanelTabs.Tab value="tags">Tags</DetailsPanelTabs.Tab>
          </DetailsPanelTabs.List>

          <DetailsPanelTabs.Panel value="details">
            <Attributes attributes={[{ title: "Resolved Access", value: resolvedAccess }]} />
          </DetailsPanelTabs.Panel>

          <DetailsPanelTabs.Panel value="permissions">
            <PathPermissions path={{ type: PathType.Folder, resourceAddress }} connection={connection} />
          </DetailsPanelTabs.Panel>

          <DetailsPanelTabs.Panel value="tags">
            <PathTags path={{ type: PathType.Folder, resourceAddress }} connection={connection} />
          </DetailsPanelTabs.Panel>
        </DetailsPanelTabs>
      )}
    </DetailsPanel>
  );
}
