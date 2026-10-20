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
import {
  CloseButton,
  Combobox,
  Group,
  InputBase,
  Loader,
  Menu,
  Text,
  TextInput,
  UnstyledButton,
  useCombobox,
} from "@mantine/core";
import { useDebouncedValue } from "@mantine/hooks";
import {
  IconAt,
  IconChevronDown,
  IconUser,
  IconUsersGroup,
} from "@tabler/icons-react";
import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import {
  useCallback,
  useEffect,
  useMemo,
  useState,
  type ComponentType,
  type ReactNode,
} from "react";
import { useAuth } from "react-oidc-context";
import { useConfig } from "../hooks/useConfig";
import { formatGroupName } from "../hooks/useGroupName";
import { formatUserName } from "../hooks/usePrincipalName";
import {
  fetchGroup,
  fetchGroups,
  fetchUser,
  fetchUsers,
  type GroupInfo,
  type UserInfo,
} from "../state/users";
import { FilterInput } from "./FilterInput";
import classes from "./PrincipalAutocomplete.module.css";
import * as React from "react";

export type PrincipalType = "user" | "group";

interface PrincipalOption {
  /** Entity id, used as the combobox option value and resolution key. */
  id: string;
  /** Value emitted through onChange when this option is selected. */
  identifier: string;
  /** Text shown in the input once the option is selected. */
  inputLabel: string;
  /** Primary line in the dropdown option. */
  primary: string;
  /** Optional secondary line in the dropdown option. */
  secondary?: string;
}

export interface PrincipalAutocompleteProps {
  error?: ReactNode;
  disabled?: boolean;
  description?: ReactNode;
  label?: ReactNode;
  placeholder?: string;
  name?: string;
  value?: string;
  /** The id_claim from the target service (e.g. "sub", "email"). Users only. */
  idClaim?: string;
  /** When set, a special "Unscoped" option with value "NULL" is shown at the top of the dropdown. */
  unscoped?: ReactNode;
  /** Whether the principal is a user or a group. Defaults to "user". */
  principalType?: PrincipalType;
  onPrincipalTypeChange?: (type: PrincipalType) => void;
  /** When set, the type indicator is shown read-only and cannot be changed. */
  lockPrincipalType?: boolean;
  onChange?: (value: string) => void;
  /** Called when the resolved state changes — true when an entity was selected from autocomplete. */
  onResolvedChange?: (resolved: boolean) => void;
}

/**
 * Autocomplete input for policy principals. Searches users or groups by display
 * name and resolves selections to the appropriate identifier. The leading type
 * indicator switches between user and group lookups.
 */
export function PrincipalAutocomplete({
  error,
  disabled,
  description,
  label = "Principal",
  placeholder = "Search by name or enter an identifier",
  name,
  value = "",
  idClaim,
  unscoped,
  principalType = "user",
  onPrincipalTypeChange,
  lockPrincipalType,
  onChange,
  onResolvedChange,
}: PrincipalAutocompleteProps) {
  const config = useConfig();
  const auth = useAuth();
  const hasUserService = !!config.endpoints.userService;
  const isGroup = principalType === "group";

  const [interaction, setInteraction] = useState<{
    search: string;
    selected: PrincipalOption | null;
  } | null>(null);
  const [initialValue] = useState(value);

  const combobox = useCombobox({
    onDropdownClose: () => combobox.resetSelectedOption(),
  });

  const initialLookup = useQuery({
    queryKey: [
      "principal-autocomplete-initial",
      principalType,
      initialValue,
      idClaim,
    ],
    queryFn: async (): Promise<PrincipalOption | null> => {
      if (isGroup) {
        try {
          const group = await fetchGroup({
            groupId: initialValue,
            config,
            user: auth.user!,
          });
          return groupOption(group);
        } catch {
          return null;
        }
      }
      if (idClaim === "email") {
        const page = await fetchUsers({
          config,
          user: auth.user!,
          email: initialValue,
          limit: 1,
        });
        const match = page.items.find((u) => u.email === initialValue) ?? null;
        return match ? userOption(match, idClaim) : null;
      }
      try {
        const user = await fetchUser({
          userId: initialValue,
          config,
          user: auth.user!,
        });
        return userOption(user, idClaim);
      } catch {
        return null;
      }
    },
    enabled:
      hasUserService &&
      !!auth.user &&
      !!initialValue &&
      initialValue !== "NULL",
    retry: false,
    staleTime: 5 * 60 * 1000,
  });

  // Until the user interacts, the initial-lookup result acts as the implicit selection.
  const selected = interaction
    ? interaction.selected
    : (initialLookup.data ?? null);
  const search =
    interaction?.search ?? (selected ? selected.inputLabel : value);

  const [debouncedSearch] = useDebouncedValue(search, 300);
  const effectiveSearch = debouncedSearch === "NULL" ? "" : debouncedSearch;

  const isResolved = !!selected;
  useEffect(() => {
    onResolvedChange?.(isResolved);
  }, [isResolved, onResolvedChange]);

  const {
    data,
    isLoading,
    isFetching,
    fetchNextPage,
    hasNextPage,
    isFetchingNextPage,
  } = useInfiniteQuery({
    queryKey: [
      "principal-autocomplete",
      principalType,
      idClaim,
      effectiveSearch,
    ],
    queryFn: async ({ pageParam }) => {
      if (isGroup) {
        const page = await fetchGroups({
          config,
          user: auth.user!,
          displayName: effectiveSearch || undefined,
          cursor: pageParam,
          limit: 20,
        });
        return {
          options: page.items.map(groupOption),
          next_cursor: page.next_cursor,
        };
      }
      const page = await fetchUsers({
        config,
        user: auth.user!,
        displayName: effectiveSearch || undefined,
        cursor: pageParam,
        limit: 20,
      });
      return {
        options: page.items.map((u) => userOption(u, idClaim)),
        next_cursor: page.next_cursor,
      };
    },
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (lastPage) => lastPage.next_cursor ?? undefined,
    enabled:
      hasUserService && !!auth.user && effectiveSearch.length >= 1 && !selected,
  });

  const searchPending =
    search !== "NULL" &&
    search.length >= 1 &&
    !selected &&
    (search !== debouncedSearch ||
      isLoading ||
      (isFetching && !isFetchingNextPage));

  const options = useMemo(
    () => data?.pages.flatMap((page) => page.options) ?? [],
    [data],
  );

  const handleOptionSubmit = useCallback(
    (optionValue: string) => {
      if (optionValue === "NULL") {
        setInteraction({ search: "NULL", selected: null });
        onChange?.("NULL");
        combobox.closeDropdown();
        return;
      }
      const option = options.find((o) => o.id === optionValue);
      if (option) {
        setInteraction({ search: option.inputLabel, selected: option });
        onChange?.(option.identifier);
      }
      combobox.closeDropdown();
    },
    [options, onChange, combobox],
  );

  const handleInputChange = useCallback(
    (event: React.ChangeEvent<HTMLInputElement>) => {
      const newValue = event.currentTarget.value;
      setInteraction({ search: newValue, selected: null });
      onChange?.(newValue);
      combobox.openDropdown();
      combobox.updateSelectedOptionIndex();
    },
    [onChange, combobox],
  );

  const handleClear = useCallback(() => {
    setInteraction({ search: "", selected: null });
    onChange?.("");
  }, [onChange]);

  const handleTypeChange = useCallback(
    (type: PrincipalType) => {
      if (type === principalType) {
        return;
      }
      onPrincipalTypeChange?.(type);
      setInteraction({ search: "", selected: null });
      onChange?.("");
      combobox.closeDropdown();
    },
    [principalType, onPrincipalTypeChange, onChange, combobox],
  );

  const handleScrollEnd = useCallback(() => {
    if (hasNextPage && !isFetchingNextPage) {
      void fetchNextPage();
    }
  }, [hasNextPage, isFetchingNextPage, fetchNextPage]);

  const typeSelector = (
    <PrincipalTypeSelector
      value={principalType}
      locked={lockPrincipalType}
      disabled={disabled}
      onChange={handleTypeChange}
    />
  );

  const sectionWidth = lockPrincipalType ? LOCKED_SECTION_WIDTH : SECTION_WIDTH;

  if (!hasUserService) {
    if (unscoped) {
      return (
        <FilterInput
          label={label}
          name={name}
          placeholder={placeholder}
          error={error}
          value={value}
          unscoped={unscoped}
          leftSection={typeSelector}
          leftSectionWidth={sectionWidth}
          onChange={onChange}
        />
      );
    }

    return (
      <TextInput
        label={label}
        name={name}
        leftSection={typeSelector}
        leftSectionWidth={sectionWidth}
        leftSectionPointerEvents={"all"}
        rightSection={!isGroup && idClaim === "email" ? <IconAt /> : null}
        description={description}
        disabled={disabled}
        error={error}
        value={value}
        onChange={(e) => onChange?.(e.currentTarget.value)}
      />
    );
  }

  const isUnscoped = search === "NULL" && !!unscoped;

  return (
    <Combobox
      store={combobox}
      withinPortal={false}
      onOptionSubmit={handleOptionSubmit}
    >
      <Combobox.Target>
        {isUnscoped ? (
          <InputBase
            component="button"
            type="button"
            pointer
            label={label}
            name={name}
            leftSection={typeSelector}
            leftSectionWidth={sectionWidth}
            leftSectionPointerEvents={"all"}
            rightSection={
              <CloseButton
                size="sm"
                aria-label="Clear principal"
                onMouseDown={(e) => e.preventDefault()}
                onClick={handleClear}
              />
            }
            description={description}
            disabled={disabled}
            error={error}
            onClick={() => combobox.openDropdown()}
            onFocus={() => combobox.openDropdown()}
            onBlur={() => combobox.closeDropdown()}
          >
            {unscoped}
          </InputBase>
        ) : (
          <TextInput
            label={label}
            name={name}
            leftSection={typeSelector}
            leftSectionWidth={sectionWidth}
            leftSectionPointerEvents={"all"}
            rightSection={
              value ? (
                <CloseButton
                  size="sm"
                  aria-label="Clear principal"
                  onMouseDown={(e) => e.preventDefault()}
                  onClick={handleClear}
                />
              ) : !isGroup && idClaim === "email" ? (
                <IconAt />
              ) : null
            }
            description={description}
            disabled={disabled}
            error={error}
            placeholder={placeholder}
            value={search}
            onChange={handleInputChange}
            onClick={() => combobox.openDropdown()}
            onFocus={() => combobox.openDropdown()}
            onBlur={() => combobox.closeDropdown()}
          />
        )}
      </Combobox.Target>

      {search !== "NULL" &&
        (searchPending ||
          options.length > 0 ||
          (unscoped && !search) ||
          (effectiveSearch.length >= 1 && !searchPending)) && (
          <Combobox.Dropdown>
            <Combobox.Options>
              {unscoped && !search && (
                <Combobox.Option value="NULL">{unscoped}</Combobox.Option>
              )}
              <EntityOptions
                options={options}
                type={principalType}
                isLoading={searchPending}
                hasQuery={effectiveSearch.length >= 1}
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
const LOCKED_SECTION_WIDTH = 88;

function PrincipalTypeSelector({
  value,
  locked,
  disabled,
  onChange,
}: {
  value: PrincipalType;
  locked?: boolean;
  disabled?: boolean;
  onChange: (type: PrincipalType) => void;
}) {
  const { label, Icon } = TYPE_META[value];

  const content = (
    <>
      <Icon size={14} />
      <Text size={"sm"} lh={1} className={classes.label}>
        {label}
      </Text>
      <span className={classes.right}>
        {!locked && <IconChevronDown size={14} />}
        <span className={classes.divider} />
      </span>
    </>
  );

  if (locked) {
    return (
      <div aria-label={`Principal type: ${label}`} className={classes.selector}>
        {content}
      </div>
    );
  }

  return (
    <Menu position={"bottom-start"} disabled={disabled}>
      <Menu.Target>
        <UnstyledButton
          type={"button"}
          disabled={disabled}
          aria-label={`Principal type: ${label}`}
          className={classes.selector}
        >
          {content}
        </UnstyledButton>
      </Menu.Target>
      <Menu.Dropdown>
        {(Object.keys(TYPE_META) as PrincipalType[]).map((type) => {
          const meta = TYPE_META[type];
          const ItemIcon = meta.Icon;
          return (
            <Menu.Item
              key={type}
              leftSection={<ItemIcon size={14} />}
              onClick={() => onChange(type)}
            >
              {meta.label}
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
  if (isLoading) {
    return (
      <Combobox.Empty>
        <Group justify="center">
          <Loader size="xs" />
          <Text size="sm">
            Loading {type === "group" ? "groups" : "users"}...
          </Text>
        </Group>
      </Combobox.Empty>
    );
  }

  if (options.length === 0) {
    if (!hasQuery) return null;
    return (
      <Combobox.Empty>
        No {type === "group" ? "groups" : "users"} found
      </Combobox.Empty>
    );
  }

  return (
    <div
      style={{ maxHeight: 250, overflowY: "auto" }}
      onScroll={(e) => {
        const target = e.currentTarget;
        const nearBottom =
          target.scrollHeight - target.scrollTop - target.clientHeight < 20;
        if (nearBottom) {
          onScrollEnd();
        }
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

function userOption(user: UserInfo, idClaim?: string): PrincipalOption {
  const identifier = idClaim === "email" && user.email ? user.email : user.id;
  return {
    id: user.id,
    identifier,
    inputLabel: formatUserName(user),
    primary: user.display_name ?? user.id,
    secondary: user.email ?? undefined,
  };
}

function groupOption(group: GroupInfo): PrincipalOption {
  return {
    id: group.id,
    identifier: group.id,
    inputLabel: formatGroupName(group),
    primary: group.display_name ?? group.id,
    secondary: group.description ?? undefined,
  };
}
