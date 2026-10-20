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

import { useMemo } from "react";
import { NavLink } from "react-router-dom";
import { Badge, Breadcrumbs, Group, Image } from "@mantine/core";
import logo from "@/assets/logo.png";
import { AppMenu } from "@/components/AppMenu";
import { ReferencesMenu } from "@/components/ReferencesMenu";
import { constructPathNavLink } from "@/hooks/usePathNavigation";
import { rootFolderPath, useActiveConnection } from "@/state/appStore";
import { getParents, getPathName } from "@/util/url";
import styles from "./Header.module.css";

export function Header() {
  const activeConnection = useActiveConnection();

  const breadcrumbs = useMemo(() => {
    if (!activeConnection) {
      return [];
    }

    const parents = getParents(activeConnection.currentFolder).slice(1);
    return [
      {
        title: getPathName(activeConnection.endpoint),
        href: constructPathNavLink(activeConnection.endpoint),
      },
    ]
      .concat(
        parents.map((parent) => ({
          title: getPathName(parent),
          href: constructPathNavLink(activeConnection.endpoint, parent),
        }))
      )
      .concat(
        activeConnection.currentFolder !== rootFolderPath
          ? [
              {
                title: getPathName(activeConnection.currentFolder),
                href: constructPathNavLink(activeConnection.endpoint, activeConnection.currentFolder),
              },
            ]
          : []
      );
  }, [activeConnection]);

  return (
    <Group p="md" h="100%" gap={30} wrap="nowrap">
      <Image src={logo} w={30} h={30} />
      <Breadcrumbs mah={60} style={{ flex: 1 }}>
        {breadcrumbs.map((breadcrumb) => (
          <Badge
            key={breadcrumb.href}
            component={NavLink}
            className={styles.breadcrumb}
            color="gray"
            flex="0 1 auto"
            fz="sm"
            lh="1.3"
            maw={450}
            to={breadcrumb.href}
            variant="light"
          >
            {breadcrumb.title}
          </Badge>
        ))}
      </Breadcrumbs>
      <ReferencesMenu />
      <AppMenu />
    </Group>
  );
}
