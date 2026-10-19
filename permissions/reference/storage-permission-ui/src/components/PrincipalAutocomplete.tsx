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

import {
  ActionIcon,
  Combobox,
  Group,
  Loader,
  Menu,
  Text,
  TextInput,
  UnstyledButton,
  useCombobox,
} from "@mantine/core";
import { useDebouncedValue } from "@mantine/hooks";
import {
  IconChevronDown,
  IconPlus,
  IconUser,
  IconUsersGroup,
} from "@tabler/icons-react";
import { useInfiniteQuery } from "@tanstack/react-query";
import {
  useCallback,
  useMemo,
  useRef,
  useState,
  type ComponentType,
} from "react";
import * as React from "react";
import { type PrincipalType } from "../api/policies.js";
import {
  fetchGroups,
  fetchUsers,
  type GroupInfo,
  type UserInfo,
} from "../api/userInfo.js";
import type { PermissionAuth } from "../types/PermissionAuth";
import type { PermissionConfig } from "../types/PermissionConfig";

interface PrincipalOption {
  /** Entity id, used as the combobox option value and resolution key. */
  id: string;
  /** Value emitted through onSubmit when this option is selected. */
  identifier: string;
  /** Primary line in the dropdown option. */
  primary: string;
  /** Optional secondary line in the dropdown option. */
  secondary?: string;
}

export interface PrincipalAutocompleteProps {
  auth: PermissionAuth;
  config: PermissionConfig;
  disabled?: boolean;
  error?: React.ReactNode;
  onSubmit: (value: string, type: PrincipalType) => void;
}

export function PrincipalAutocomplete({
  auth,
  config,
  disabled,
  error,
  onSubmit,
}: PrincipalAutocompleteProps) {
  const hasUserService = !!config.userInfoUrl;
  const [principalType, setPrincipalType] = useState<PrincipalType>("user");
  const isGroup = principalType === "group";
  const [search, setSearch] = useState("");
  const [debouncedSearch] = useDebouncedValue(search, 300);
  const selectedRef = useRef(false);

  const combobox = useCombobox({
    onDropdownClose: () => combobox.resetSelectedOption(),
  });

  const {
    data,
    isLoading,
    isFetching,
    fetchNextPage,
    hasNextPage,
    isFetchingNextPage,
  } = useInfiniteQuery({
    queryKey: ["principal-autocomplete", principalType, debouncedSearch],
    queryFn: async ({ pageParam }) => {
      if (isGroup) {
        const page = await fetchGroups({
          auth,
          config,
          displayName: debouncedSearch || undefined,
          cursor: pageParam,
          limit: 20,
        });
        return {
          options: page.items.map(groupOption),
          next_cursor: page.next_cursor,
        };
      }
      const page = await fetchUsers({
        auth,
        config,
        displayName: debouncedSearch || undefined,
        cursor: pageParam,
        limit: 20,
      });
      return {
        options: page.items.map(userOption),
        next_cursor: page.next_cursor,
      };
    },
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (lastPage) => lastPage.next_cursor ?? undefined,
    enabled:
      hasUserService && debouncedSearch.length >= 1 && !selectedRef.current,
  });

  const searchPending =
    search.length >= 1 &&
    !selectedRef.current &&
    (search !== debouncedSearch ||
      isLoading ||
      (isFetching && !isFetchingNextPage));

  const options = useMemo(
    () => data?.pages.flatMap((page) => page.options) ?? [],
    [data],
  );

  const handleOptionSubmit = useCallback(
    (optionValue: string) => {
      const option = options.find((o) => o.id === optionValue);
      if (option) {
        selectedRef.current = true;
        setSearch("");
        onSubmit(option.identifier, principalType);
      }
      combobox.closeDropdown();
    },
    [options, onSubmit, principalType, combobox],
  );

  const handleInputChange = useCallback(
    (event: React.ChangeEvent<HTMLInputElement>) => {
      selectedRef.current = false;
      setSearch(event.currentTarget.value);
      combobox.openDropdown();
      combobox.updateSelectedOptionIndex();
    },
    [combobox],
  );

  const submitSearch = useCallback(() => {
    if (search.trim()) {
      selectedRef.current = false;
      onSubmit(search.trim(), principalType);
      setSearch("");
      combobox.closeDropdown();
    }
  }, [search, onSubmit, principalType, combobox]);

  function handleInputKeyDown(event: React.KeyboardEvent<HTMLInputElement>) {
    if (event.key === "Enter") {
      event.preventDefault();
      event.stopPropagation();
      submitSearch();
    }
  }

  function handleAddClick(event: React.MouseEvent<HTMLButtonElement>) {
    event.preventDefault();
    submitSearch();
  }

  const handleTypeChange = useCallback(
    (type: PrincipalType) => {
      if (type === principalType) {
        return;
      }
      setPrincipalType(type);
      selectedRef.current = false;
      setSearch("");
      combobox.closeDropdown();
    },
    [principalType, combobox],
  );

  const handleScrollEnd = useCallback(() => {
    if (hasNextPage && !isFetchingNextPage) {
      void fetchNextPage();
    }
  }, [hasNextPage, isFetchingNextPage, fetchNextPage]);

  const typeSelector = (
    <PrincipalTypeSelector
      value={principalType}
      disabled={disabled}
      onChange={handleTypeChange}
    />
  );

  const addButton = (
    <ActionIcon
      aria-label={"Add principal"}
      variant={"transparent"}
      color={"green"}
      onClick={handleAddClick}
    >
      <IconPlus size={20} />
    </ActionIcon>
  );

  if (!hasUserService) {
    return (
      <TextInput
        disabled={disabled}
        mr={7}
        error={error}
        placeholder={"Add new principal..."}
        value={search}
        leftSection={typeSelector}
        leftSectionWidth={SECTION_WIDTH}
        leftSectionPointerEvents={"all"}
        rightSection={addButton}
        rightSectionPointerEvents={"auto"}
        onKeyDown={handleInputKeyDown}
        onChange={handleInputChange}
      />
    );
  }

  return (
    <Combobox
      store={combobox}
      withinPortal={false}
      onOptionSubmit={handleOptionSubmit}
    >
      <Combobox.Target>
        <TextInput
          disabled={disabled}
          mr={7}
          error={error}
          placeholder={"Add new principal..."}
          value={search}
          leftSection={typeSelector}
          leftSectionWidth={SECTION_WIDTH}
          leftSectionPointerEvents={"all"}
          rightSection={addButton}
          rightSectionPointerEvents={"auto"}
          onKeyDown={handleInputKeyDown}
          onChange={handleInputChange}
          onClick={() => combobox.openDropdown()}
          onFocus={() => combobox.openDropdown()}
          onBlur={() => combobox.closeDropdown()}
        />
      </Combobox.Target>

      {(searchPending ||
        options.length > 0 ||
        (debouncedSearch.length >= 1 && !searchPending)) && (
        <Combobox.Dropdown>
          <Combobox.Options>
            <EntityOptions
              options={options}
              type={principalType}
              isLoading={searchPending}
              hasQuery={debouncedSearch.length >= 1}
              hasNextPage={hasNextPage}
              isFetchingNextPage={isFetchingNextPage}
              onScrollEnd={handleScrollEnd}
            />
          </Combobox.Options>
        </Combobox.Dropdown>
      )}
    </Combobox>
  );
}

const TYPE_META: Record<
  PrincipalType,
  { label: string; Icon: ComponentType<{ size?: number }> }
> = {
  user: { label: "User", Icon: IconUser },
  group: { label: "Group", Icon: IconUsersGroup },
};

/** Fixed so the input text keeps the same left position across types. */
const SECTION_WIDTH = 110;

function PrincipalTypeSelector({
  value,
  disabled,
  onChange,
}: {
  value: PrincipalType;
  disabled?: boolean;
  onChange: (type: PrincipalType) => void;
}) {
  const { label, Icon } = TYPE_META[value];

  return (
    <Menu position={"bottom-start"} disabled={disabled}>
      <Menu.Target>
        <UnstyledButton
          type={"button"}
          disabled={disabled}
          aria-label={`Principal type: ${label}`}
          style={{
            display: "flex",
            alignItems: "center",
            width: "100%",
            height: "100%",
            paddingLeft: "calc(var(--input-padding) - 4px)",
            paddingRight: "var(--mantine-spacing-sm)",
            color: "var(--mantine-color-text)",
          }}
          onMouseDown={(event) => event.stopPropagation()}
          onClick={(event) => event.stopPropagation()}
        >
          <Icon size={14} />
          <Text
            size={"sm"}
            lh={1}
            style={{
              marginInlineStart: "0.5rem",
              whiteSpace: "nowrap",
              fontSize: "10pt",
            }}
          >
            {label}
          </Text>
          <span
            style={{
              marginInlineStart: "auto",
              display: "flex",
              alignItems: "center",
              alignSelf: "stretch",
              gap: 6,
            }}
          >
            <IconChevronDown size={14} />
            <span
              style={{
                alignSelf: "stretch",
                borderRight: "1px solid var(--mantine-color-default-border)",
              }}
            />
          </span>
        </UnstyledButton>
      </Menu.Target>
      <Menu.Dropdown>
        {(Object.keys(TYPE_META) as PrincipalType[]).map((type) => {
          const ItemIcon = TYPE_META[type].Icon;
          return (
            <Menu.Item
              key={type}
              leftSection={<ItemIcon size={14} />}
              onClick={() => onChange(type)}
            >
              {TYPE_META[type].label}
            </Menu.Item>
          );
        })}
      </Menu.Dropdown>
    </Menu>
  );
}

function EntityOptions({
  options,
  type,
  isLoading,
  hasQuery,
  hasNextPage,
  isFetchingNextPage,
  onScrollEnd,
}: {
  options: PrincipalOption[];
  type: PrincipalType;
  isLoading: boolean;
  hasQuery: boolean;
  hasNextPage: boolean;
  isFetchingNextPage: boolean;
  onScrollEnd: () => void;
}) {
  const noun = type === "group" ? "groups" : "users";

  if (isLoading) {
    return (
      <Combobox.Empty>
        <Group justify="center">
          <Loader size="xs" />
          <Text size="sm">Loading {noun}...</Text>
        </Group>
      </Combobox.Empty>
    );
  }

  if (options.length === 0) {
    if (!hasQuery) return null;
    return <Combobox.Empty>No {noun} found</Combobox.Empty>;
  }

  return (
    <div
      style={{ maxHeight: 250, overflowY: "auto" }}
      onScroll={(e) => {
        const target = e.currentTarget;
        const nearBottom =
          target.scrollHeight - target.scrollTop - target.clientHeight < 20;
        if (nearBottom) onScrollEnd();
      }}
    >
      {options.map((option) => (
        <Combobox.Option key={option.id} value={option.id}>
          <Text size="sm" fw={500}>
            {option.primary}
          </Text>
          {option.secondary && (
            <Text size="xs" c="dimmed">
              {option.secondary}
            </Text>
          )}
        </Combobox.Option>
      ))}
      {hasNextPage && (
        <Combobox.Empty>
          {isFetchingNextPage ? (
            <Loader size="xs" />
          ) : (
            <Text size="xs" c="dimmed">
              Scroll for more results
            </Text>
          )}
        </Combobox.Empty>
      )}
    </div>
  );
}

function userOption(user: UserInfo): PrincipalOption {
  return {
    id: user.id,
    identifier: user.id,
    primary: user.display_name ?? user.id,
    secondary: user.email ?? undefined,
  };
}

function groupOption(group: GroupInfo): PrincipalOption {
  return {
    id: group.id,
    identifier: group.id,
    primary: group.display_name ?? group.id,
    secondary: group.description ?? undefined,
  };
}
