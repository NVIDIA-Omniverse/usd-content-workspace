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
import { Button, Group, TextInput } from "@mantine/core";
import { IconChevronLeft, IconChevronRight } from "@tabler/icons-react";
import * as React from "react";
import { NavLink, useNavigate, useSearchParams } from "react-router-dom";

export interface NavPaginationProps {
  page: number;
}

export function NavPagination({ page }: NavPaginationProps) {
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();

  const prevPage = getPageParams(page - 1, searchParams);
  const nextPage = getPageParams(page + 1, searchParams);

  const handleKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Enter") {
      const page = event.currentTarget.valueAsNumber;
      if (isNaN(page)) {
        return;
      }

      const pageParams = getPageParams(page, searchParams);
      navigate(`?${pageParams}`);
    }
  };

  return (
    <Group justify={"center"} align={"flex-end"}>
      <Button
        component={NavLink}
        disabled={page <= 1}
        to={`?${prevPage}`}
        title={page > 1 ? `Go to page ${page - 1}` : ""}
        variant={"default"}
      >
        <IconChevronLeft />
      </Button>

      <TextInput
        defaultValue={page}
        label={"Page"}
        type={"number"}
        w={100}
        onKeyDown={handleKeyDown}
      />

      <Button
        component={NavLink}
        to={`?${nextPage}`}
        title={`Go to page ${page + 1}`}
        variant={"default"}
      >
        <IconChevronRight />
      </Button>
    </Group>
  );
}

const getPageParams = (
  page: number,
  base: URLSearchParams,
): URLSearchParams => {
  const params = new URLSearchParams(base);
  params.set("page", page.toString());
  return params;
};
