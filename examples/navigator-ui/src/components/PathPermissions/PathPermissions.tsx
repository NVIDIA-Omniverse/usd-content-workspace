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

// @ts-ignore
import { PermissionForm, ResourceAddress } from "@omniverse/storage-permission-ui";
import { useQuery } from "@tanstack/react-query";
import { Box, Loader } from "@mantine/core";
import { LoaderError } from "@/components/LoaderError";
import { useAppConfig } from "@/hooks/useAppConfig";
import { useServiceApi } from "@/hooks/useServiceApi";
import { getApiOptions } from "@/services/api";
import { PathShortInfo, PathType } from "@/state/path";
import { Connection } from "@/state/slices/connectionSlice";
import { PermissionForm as PermissionStubForm } from "@/stubs/@omniverse/storage-permission-ui";

export interface PathPermissionsProps {
  path: PathShortInfo;
  connection: Connection;
}

export function PathPermissions({ path, connection }: PathPermissionsProps) {
  const { requestTimeout } = useAppConfig();
  const { getApi } = useServiceApi();
  const permission = useQuery({
    queryKey: [connection.info.discovery, "discovery", "permission"],
    queryFn: async () => {
      const options = getApiOptions(connection, { requestTimeout });
      const api = await getApi(connection.info.discovery, "permission", options);
      return api?.baseUrl ?? "";
    },
  });

  const userinfo = useQuery({
    queryKey: [connection.info.discovery, "discovery", "userInfo"],
    queryFn: async () => {
      const options = getApiOptions(connection, { requestTimeout });
      const api = await getApi(connection.info.discovery, "userinfo", options);
      return api?.baseUrl ?? "";
    },
  });

  if (permission.isLoading || userinfo.isLoading) {
    return <Loader p="xs" />;
  }

  if (permission.isError) {
    return <LoaderError title="Failed to load permissions">{permission.error.message}</LoaderError>;
  }

  if (userinfo.isError) {
    return <LoaderError title="Failed to load userinfo">{userinfo.error.message}</LoaderError>;
  }

  if (!connection.info.auth || !permission.isSuccess || !userinfo.isSuccess) {
    return null;
  }

  if (!permission.data) {
    return <PermissionStubForm />;
  }

  const resource = convertToResourceAddress(path);
  return (
    <Box p="xs">
      <PermissionForm
        auth={{ accessToken: connection.info.auth.accessToken, idToken: connection.info.auth.idToken }}
        config={{ url: permission.data, userInfoUrl: userinfo.data }}
        resource={resource}
      />
    </Box>
  );
}

function convertToResourceAddress(path: PathShortInfo): ResourceAddress {
  let type: ResourceAddress["type"];
  switch (path.type) {
    case PathType.File:
      type = "object";
      break;
    case PathType.Folder:
    case PathType.TopLevelAddress:
      type = "folder";
      break;
  }
  return {
    type,
    address: path.resourceAddress,
  };
}
