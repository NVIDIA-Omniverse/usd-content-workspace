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

import { useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import fileImage from "@/assets/file.png";
import folderImage from "@/assets/folder.png";
import serverImage from "@/assets/server.png";
import { useAppConfig } from "@/hooks/useAppConfig";
import { useServiceApi } from "@/hooks/useServiceApi";
import { create, getApiOptions } from "@/services/api";
import { fileObjectService } from "@/services/storage/fileObjectService";
import { thumbnailService } from "@/services/thumbnailService";
import { Path, PathType } from "@/state/path";
import { Connection } from "@/state/slices/connectionSlice";
import { decodePathName } from "@/util/url";

export const DEFAULT_IMAGES = {
  [PathType.Folder]: folderImage,
  [PathType.File]: fileImage,
  [PathType.TopLevelAddress]: serverImage,
} as const;

const THUMBNAIL_SIZE = 256;
const THUMBNAIL_QUERY_DEBOUNCE_MS = 150;

interface DebouncedThumbnailQueryInputs {
  connection: Connection;
  resourceAddress: string;
}

export function usePathThumbnail(path: Path, connection: Connection): string {
  // Files and folders both use resourceAddress. Top-level addresses have no thumbnail.
  const resourceAddress =
    path.type === PathType.File || path.type === PathType.Folder ? path.resourceAddress : undefined;
  const [debouncedQueryInputs, setDebouncedQueryInputs] = useState<DebouncedThumbnailQueryInputs>();
  const { requestTimeout } = useAppConfig();
  const { getApi, createServiceApi } = useServiceApi();
  const queryEnabled =
    debouncedQueryInputs?.connection === connection && debouncedQueryInputs.resourceAddress === resourceAddress;

  useEffect(() => {
    setDebouncedQueryInputs(undefined);

    if (!resourceAddress) {
      return;
    }

    const timeout = window.setTimeout(() => {
      setDebouncedQueryInputs({ connection, resourceAddress });
    }, THUMBNAIL_QUERY_DEBOUNCE_MS);

    return () => {
      window.clearTimeout(timeout);
    };
  }, [resourceAddress, connection]);

  // Step 1: Discover the thumbnail service REST endpoint.
  // Returns null when the service is not available; undefined while loading.
  const { data: thumbnailServiceUrl } = useQuery<string | null>({
    queryKey: ["thumbnail-service-discovery", connection.info.discovery],
    enabled: queryEnabled && !!resourceAddress,
    queryFn: async () => {
      try {
        const apiOptions = getApiOptions(connection, { requestTimeout });
        const thumbnail = await getApi(connection.info.discovery, "thumbnail", apiOptions);
        return thumbnail?.url ?? null;
      } catch {
        return null;
      }
    },
  });

  // Step 2: Query the thumbnail service for this path's resource identity.
  // Wait for step 1 — if we run before thumbnailServiceUrl is known we'd cache null and never retry.
  const { data: thumbnailResource } = useQuery<string | null>({
    queryKey: thumbnailResourceQueryKey(connection.endpoint, resourceAddress),
    enabled: queryEnabled && !!thumbnailServiceUrl && !!resourceAddress,
    retry: false,
    staleTime: 5 * 60 * 1000,
    queryFn: async ({ signal }) => {
      const apiOptions = getApiOptions(connection, { requestTimeout });
      const api = create(thumbnailServiceUrl!, apiOptions);
      const response = await thumbnailService(api).getThumbnails(resourceAddress!, {
        signal,
        width: THUMBNAIL_SIZE,
        height: THUMBNAIL_SIZE,
      });
      const thumbnail = response.thumbnails[0];
      return thumbnail?.thumbnail.resourceIdentity ?? null;
    },
  });

  // Step 3: Download the thumbnail blob by resource identity.
  const { data: thumbnailBlob } = useQuery<Blob | undefined, Error>({
    queryKey: [connection.endpoint, thumbnailResource, "thumbnail", "blob"],
    enabled: queryEnabled && !!thumbnailResource,
    retry: false,
    queryFn: async ({ signal }) => {
      const apiOptions = getApiOptions(connection, { requestTimeout });
      const api = await createServiceApi(connection.info.discovery, "fileobject", apiOptions);
      if (!api) {
        return undefined;
      }
      const response = await fileObjectService(api).download(thumbnailResource!, "identity", { signal });
      if (!response) {
        return undefined;
      }
      return await response.blob();
    },
  });

  const urlRef = useRef<string | undefined>(undefined);
  const [thumbnailUrl, setThumbnailUrl] = useState<string>();

  useEffect(() => {
    if (urlRef.current) {
      URL.revokeObjectURL(urlRef.current);
      urlRef.current = undefined;
    }

    if (thumbnailBlob) {
      const url = URL.createObjectURL(thumbnailBlob);
      urlRef.current = url;
      setThumbnailUrl(url);
    } else {
      setThumbnailUrl(undefined);
    }
  }, [thumbnailBlob]);

  useEffect(() => {
    return () => {
      if (urlRef.current) {
        URL.revokeObjectURL(urlRef.current);
      }
    };
  }, []);

  return thumbnailUrl ?? DEFAULT_IMAGES[path.type];
}

export function thumbnailResourceQueryKey(endpoint: string, resourceAddress: string | undefined) {
  return [endpoint, resourceAddress ? decodePathName(resourceAddress) : "", "thumbnail", "resource"] as const;
}
