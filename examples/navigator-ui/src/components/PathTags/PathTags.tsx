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
import { useQueryClient } from "@tanstack/react-query";
import { Alert, Box, Loader } from "@mantine/core";
import { TagFormTable } from "@/components/PathTags/TagFormTable";
import { usePathMetadata } from "@/hooks/usePathMetadata";
import { PathShortInfo } from "@/state/path";
import { Connection } from "@/state/slices/connectionSlice";

export interface PathTagsProps {
  path: PathShortInfo;
  connection: Connection;
}

export function PathTags({ path, connection }: PathTagsProps) {
  const queryClient = useQueryClient();
  const metadataQuery = usePathMetadata(path.resourceAddress, connection);

  if (metadataQuery.isLoading) {
    return <Loader />;
  }

  if (metadataQuery.isError) {
    return (
      <Alert color="red" title="Failed to load tags">
        {metadataQuery.error instanceof Error ? metadataQuery.error.message : "Failed to load tags"}
      </Alert>
    );
  }

  if (!metadataQuery.isSuccess) {
    return null;
  }

  function resetForm() {
    void queryClient.invalidateQueries({
      queryKey: [connection.endpoint, path.resourceAddress, "metadata", "tags"],
    });
  }

  return (
    <Box p="xs">
      <TagFormTable
        key={path.resourceAddress}
        connection={connection}
        path={path}
        tags={metadataQuery.data.tags.value}
        etag={metadataQuery.data.tags.etag}
        onReset={resetForm}
      />
    </Box>
  );
}
