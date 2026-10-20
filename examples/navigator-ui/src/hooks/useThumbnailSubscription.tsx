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

import { useEffect } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Connection } from "@/state/appStore";
import { PathSubscription } from "./usePathSubscription";
import { thumbnailResourceQueryKey } from "./usePathThumbnail";

export function useThumbnailSubscription(connection: Connection, pathSubscription: PathSubscription) {
  const queryClient = useQueryClient();

  useEffect(() => {
    return pathSubscription.onEvent((event) => {
      if (event.eventType !== "omni.thumbnail.associated") {
        return;
      }

      queryClient.setQueryData(
        thumbnailResourceQueryKey(connection.endpoint, event.message.associationTarget.resourceAddress),
        event.message.thumbnail.resourceIdentity
      );
    });
  }, [connection.endpoint, pathSubscription, queryClient]);
}
