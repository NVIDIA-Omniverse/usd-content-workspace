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

import { IconHistory } from "@tabler/icons-react";
import { Accordion } from "@mantine/core";
import { Attributes } from "@/components/Attributes";
import { DetailsPanel } from "@/components/DetailsPanel";
import { DetailsPanelTabs } from "@/components/DetailsPanelTabs";
import { FileVersions } from "@/components/FileVersions";
import { PathPermissions } from "@/components/PathPermissions";
import { PathTags } from "@/components/PathTags";
import { PathText } from "@/components/PathText";
import { usePathMetadata } from "@/hooks/usePathMetadata";
import { useAppStore } from "@/state/appStore";
import { Path, PathType } from "@/state/path";
import { DetailsPanelTab } from "@/state/slices/applicationSlice";
import { Connection } from "@/state/slices/connectionSlice";
import styles from "./PathDetails.module.css";

export interface PathDetailsProps {
  path: Path;
  connection: Connection;
}

export function PathDetails({ path, connection }: PathDetailsProps) {
  const metadata = usePathMetadata(path.resourceAddress, connection);
  const resolvedAccess = metadata.data?.acl.value.toSorted().join(", ");

  const detailsPanelTab = useAppStore((state) => state.detailsPanelTab);
  const setDetailsPanelTab = useAppStore((state) => state.setDetailsPanelTab);

  return (
    <DetailsPanel>
      <DetailsPanel.Title title={path.name}>{path.name}</DetailsPanel.Title>

      <PathText bg="dark" px="md" pb="xs" title={path.resourceAddress}>
        {path.resourceAddress}
      </PathText>

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
          <Attributes
            attributes={
              path.type === PathType.File
                ? [
                    { title: "Type", value: path.displayedType },
                    { title: "Size", value: path.size },
                    { title: "Modified", value: path.modified },
                    { title: "Resolved Access", value: resolvedAccess },
                  ]
                : [
                    { title: "Type", value: path.displayedType },
                    { title: "Resolved Access", value: resolvedAccess },
                  ]
            }
          />

          {path.type === PathType.File && (
            <Accordion className={styles.accordion} multiple defaultValue={["versioning"]} variant="separated" m="md">
              <Accordion.Item value="versioning">
                <Accordion.Control icon={<IconHistory />}>Versions</Accordion.Control>
                <Accordion.Panel>
                  <FileVersions file={path} connection={connection} />
                </Accordion.Panel>
              </Accordion.Item>
            </Accordion>
          )}
        </DetailsPanelTabs.Panel>

        <DetailsPanelTabs.Panel value="permissions">
          <PathPermissions path={path} connection={connection} />
        </DetailsPanelTabs.Panel>

        <DetailsPanelTabs.Panel value="tags">
          <PathTags path={path} connection={connection} />
        </DetailsPanelTabs.Panel>
      </DetailsPanelTabs>
    </DetailsPanel>
  );
}
