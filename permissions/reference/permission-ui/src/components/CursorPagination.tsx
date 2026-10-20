/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
import { Button, Group } from "@mantine/core";
import { IconChevronLeft, IconChevronRight } from "@tabler/icons-react";

export interface CursorPaginationProps {
  hasPrevious: boolean;
  hasNext: boolean;
  onPrevious: () => void;
  onNext: () => void;
}

export function CursorPagination({
  hasPrevious,
  hasNext,
  onPrevious,
  onNext,
}: CursorPaginationProps) {
  if (!hasPrevious && !hasNext) return null;

  return (
    <Group justify={"center"}>
      <Button
        variant={"default"}
        disabled={!hasPrevious}
        leftSection={<IconChevronLeft size={16} />}
        onClick={onPrevious}
      >
        Previous
      </Button>
      <Button
        variant={"default"}
        disabled={!hasNext}
        rightSection={<IconChevronRight size={16} />}
        onClick={onNext}
      >
        Next
      </Button>
    </Group>
  );
}
