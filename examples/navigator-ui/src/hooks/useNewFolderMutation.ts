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

import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useAppConfig } from "@/hooks/useAppConfig";
import { folderContentQueryKey } from "@/hooks/useFolderContentQuery";
import { getApiOptions } from "@/services/api";
import { fileFolderService } from "@/services/storage";
import { useActiveConnection } from "@/state/appStore";
import { join } from "@/util/url";
import { useNotification } from "./useNotification";
import { useServiceApi } from "./useServiceApi";

export function useNewFolderMutation() {
  const activeConnection = useActiveConnection();
  const queryClient = useQueryClient();
  const notification = useNotification();
  const { requestTimeout } = useAppConfig();
  const { createServiceApi } = useServiceApi();

  return useMutation({
    mutationFn: async (name: string) => {
      if (!activeConnection) {
        throw new Error("No active connection is available for creating a new folder.");
      }
      const options = getApiOptions(activeConnection, { requestTimeout });
      const api = await createServiceApi(activeConnection.info.discovery, "filefolder", options);
      if (!api) {
        throw new Error("No active connection is available for creating a new folder.");
      }

      const currentFolder = activeConnection.currentFolder;
      const newFolderAddress = join(currentFolder, encodeURIComponent(name));
      await fileFolderService(api).createFolder(newFolderAddress);

      await queryClient.invalidateQueries({
        queryKey: folderContentQueryKey(activeConnection.endpoint, currentFolder),
      });

      notification.show({
        type: "success",
        message: `New folder '${name}' has been created.`,
      });
    },
  });
}
