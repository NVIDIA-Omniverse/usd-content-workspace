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
  Accordion,
  Alert,
  Center,
  Loader,
  Stack,
  Text,
  UnstyledButton,
} from "@mantine/core";
import {
  IconAlertTriangleFilled,
  IconChevronDown,
  IconForbid,
} from "@tabler/icons-react";
import { useId, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { checkUserAccess } from "../api/authorization.js";
import { getMetadata } from "../api/metadata.js";
import { getGlobalPolicies, getPermissions } from "../api/policies.js";
import { getResolvedPermissions } from "../api/resolved.js";
import { fetchCurrentUser } from "../api/userInfo.js";
import type { PermissionAuth } from "../types/PermissionAuth.js";
import type { PermissionConfig } from "../types/PermissionConfig.js";
import type { ResourceAddress } from "../types/ResourceAddress.js";
import { getTokenPrincipal } from "../utils/auth.js";
import { PermissionFormTable } from "./PermissionFormTable.js";
import { ResolvedPermissionsTable } from "./ResolvedPermissionsTable.js";
import Placeholder from "./Placeholder.js";

export interface PermissionFormProps {
  auth: PermissionAuth;
  config: PermissionConfig;
  resource: ResourceAddress;
}

export function PermissionForm({
  auth,
  config,
  resource,
}: PermissionFormProps) {
  const queryClient = useQueryClient();
  const permissions = useQuery({
    queryKey: [resource.address, "permissions", config.url, auth.accessToken],
    queryFn: async () => getPermissions({ auth, config, resource }),
    staleTime: 0,
    refetchOnMount: "always",
    refetchOnReconnect: false,
    refetchOnWindowFocus: false,
  });

  const hasUserService = !!config.userInfoUrl;

  const tokenPrincipal = useQuery({
    queryKey: ["permissions-metadata", config.url, auth.accessToken],
    queryFn: async () => {
      const metadata = await getMetadata({ auth, config });
      return getTokenPrincipal(auth.idToken, metadata);
    },
    enabled: !hasUserService,
  });

  const currentUserInfo = useQuery({
    queryKey: [
      "permissions-current-user",
      config.userInfoUrl,
      auth.accessToken,
    ],
    queryFn: async () => fetchCurrentUser({ auth, config }),
    enabled: hasUserService,
    retry: false,
  });

  const userAccess = useQuery({
    queryKey: [
      resource.address,
      "permissions-user-access",
      config.url,
      auth.accessToken,
    ],
    queryFn: async () => await checkUserAccess({ auth, config, resource }),
  });

  const resolved = useQuery({
    queryKey: [
      resource.address,
      "resolved-permissions",
      config.url,
      config.userInfoUrl,
      auth.accessToken,
    ],
    queryFn: async () => getResolvedPermissions({ auth, config, resource }),
    staleTime: 0,
    refetchOnMount: "always",
    refetchOnReconnect: false,
    refetchOnWindowFocus: false,
  });

  const globalPolicies = useQuery({
    queryKey: ["global-policies", config.url, auth.accessToken],
    queryFn: async () => getGlobalPolicies({ auth, config }),
    staleTime: 0,
    refetchOnMount: "always",
    refetchOnReconnect: false,
    refetchOnWindowFocus: false,
  });

  const userQuery = hasUserService ? currentUserInfo : tokenPrincipal;

  if (permissions.isLoading || userQuery.isLoading || userAccess.isLoading) {
    return <Loader />;
  }

  if (permissions.isError) {
    return (
      <Alert title={"Failed to load permissions"} color={"red"}>
        {permissions.error.message}
      </Alert>
    );
  }

  if (userQuery.isError) {
    return (
      <Alert title={"Failed to load user identity"} color={"red"}>
        {userQuery.error.message}
      </Alert>
    );
  }

  if (userAccess.isError) {
    return (
      <Alert title={"Failed to load user access"} color={"red"}>
        {userAccess.error.message}
      </Alert>
    );
  }

  function resetForm() {
    void queryClient.invalidateQueries({
      queryKey: [resource.address, "permissions"],
    });
  }

  if (!permissions.isSuccess || !userAccess.isSuccess || !userQuery.isSuccess) {
    return null;
  }

  if (!userAccess.data.canViewPermissions) {
    return (
      <Center>
        <Placeholder icon={<IconForbid size={64} />}>
          You're not allowed to view permissions for this resource.
        </Placeholder>
      </Center>
    );
  }

  const userId = currentUserInfo.data?.id ?? tokenPrincipal.data ?? "";

  const globalError = globalPolicies.isError;
  const hasGlobalPolicies = (globalPolicies.data?.policies.length ?? 0) > 0;
  const hasUnsupportedPolicies = permissions.data.unsupported.length > 0;
  const hasUnresolvableGlobalPolicies =
    (globalPolicies.data?.unresolvable.length ?? 0) > 0;

  const errorWarning = (
    <GlobalAccessWarning
      title={"Global policies couldn't be checked"}
      description={
        "Global policies that may apply to this resource couldn't be loaded, so some access rules might not be shown here."
      }
    />
  );

  let assignedWarning = null;
  if (globalError) {
    assignedWarning = errorWarning;
  } else if (hasGlobalPolicies || hasUnsupportedPolicies) {
    assignedWarning = (
      <GlobalAccessWarning
        title={"Some policies can't be managed here"}
        description={
          "Some policies apply to this resource but can't be displayed or managed in this panel."
        }
      />
    );
  }

  let resolvedWarning = null;
  if (globalError) {
    resolvedWarning = errorWarning;
  } else if (hasUnresolvableGlobalPolicies) {
    resolvedWarning = (
      <GlobalAccessWarning
        title={"Some access rules aren't shown here"}
        description={
          "Some global rules affect this resource but can't be shown here, so the actual access may differ from what's listed."
        }
      />
    );
  }

  const assignedContent = permissions.isFetching ? (
    <Center p={"md"}>
      <Loader />
    </Center>
  ) : (
    <PermissionFormTable
      key={permissions.dataUpdatedAt}
      auth={auth}
      config={config}
      data={permissions.data}
      resource={resource}
      user={userId}
      userAccess={userAccess.data}
      onReset={resetForm}
    />
  );

  let resolvedInner;
  if (resolved.isFetching) {
    resolvedInner = (
      <Center p={"md"}>
        <Loader />
      </Center>
    );
  } else if (resolved.isError) {
    resolvedInner = (
      <Alert title={"Failed to load resolved permissions"} color={"red"}>
        {resolved.error.message}
      </Alert>
    );
  } else if (
    !resolved.data ||
    Object.keys(resolved.data.entries).length === 0
  ) {
    resolvedInner = (
      <Text c={"dimmed"} size={"sm"}>
        No effective permissions found for this resource.
      </Text>
    );
  } else {
    resolvedInner = (
      <Stack gap={"xs"}>
        <ResolvedPermissionsTable
          auth={auth}
          config={config}
          data={resolved.data}
          resource={resource}
        />
        {!config.userInfoUrl && (
          <Text c={"dimmed"} size={"xs"}>
            Group-inherited permissions are not shown because the user
            information service is not configured.
          </Text>
        )}
      </Stack>
    );
  }

  const resolvedContent = resolvedInner;

  return (
    <Accordion
      multiple
      variant={"separated"}
      chevronPosition={"right"}
      defaultValue={["assigned", "resolved"]}
      style={{ minWidth: 320 }}
      styles={{
        control: {
          backgroundColor: "var(--mantine-color-default-hover)",
          paddingLeft:
            "calc(var(--mantine-spacing-md) + var(--mantine-spacing-sm))",
        },
        content: {
          paddingLeft: "var(--mantine-spacing-md)",
          paddingRight: "var(--mantine-spacing-md)",
          paddingTop: "var(--mantine-spacing-md)",
          paddingBottom: "var(--mantine-spacing-md)",
        },
        label: {
          textTransform: "uppercase",
          fontSize: "8pt",
          fontWeight: 600,
        },
      }}
    >
      <Accordion.Item value={"assigned"}>
        <Accordion.Control>Assigned to selection</Accordion.Control>
        <Accordion.Panel>
          {assignedWarning}
          {assignedContent}
        </Accordion.Panel>
      </Accordion.Item>

      <Accordion.Item value={"resolved"}>
        <Accordion.Control>Resolved</Accordion.Control>
        <Accordion.Panel>
          {resolvedWarning}
          {resolvedContent}
        </Accordion.Panel>
      </Accordion.Item>
    </Accordion>
  );
}

interface GlobalAccessWarningProps {
  title: string;
  description: string;
}

/**
 * Panel banner that alerts the user about global policies affecting the current
 * resource, either because some cannot be viewed or managed here or because they
 * could not be loaded. A chevron toggles the explanatory text, and each instance
 * keeps its own expanded state so the two panels can be toggled independently.
 *
 * @param props.title Short headline shown in the banner header.
 * @param props.description Explanatory text revealed when the banner is expanded.
 */
function GlobalAccessWarning({ title, description }: GlobalAccessWarningProps) {
  const [expanded, setExpanded] = useState(true);
  const descriptionId = useId();

  return (
    <Alert
      color={"yellow"}
      radius={0}
      icon={<IconAlertTriangleFilled size={16} />}
      title={
        <>
          <Text size={"xs"} fw={600} c={"inherit"}>
            {title}
          </Text>
          <UnstyledButton
            aria-label={"Toggle details"}
            aria-expanded={expanded}
            aria-controls={descriptionId}
            onClick={() => setExpanded((value) => !value)}
            style={{ display: "flex", alignItems: "center", padding: "2px 0" }}
          >
            <IconChevronDown
              size={15}
              style={{
                transform: expanded ? "rotate(180deg)" : undefined,
                transition: "transform 150ms ease",
              }}
            />
          </UnstyledButton>
        </>
      }
      styles={{
        root: {
          marginTop: "calc(-1 * var(--mantine-spacing-md))",
          marginLeft: "calc(-1 * var(--mantine-spacing-md))",
          marginRight: "calc(-1 * var(--mantine-spacing-md))",
          marginBottom: "var(--mantine-spacing-md)",
          paddingTop: "var(--mantine-spacing-xs)",
          paddingBottom: "var(--mantine-spacing-xs)",
          paddingLeft: "6px",
          paddingRight: "var(--mantine-spacing-md)",
          borderTop: "1px solid var(--item-border-color)",
          borderBottom: "1px solid var(--item-border-color)",
        },
        icon: { width: 16, minWidth: 16, height: 16, marginInlineEnd: "6px" },
        label: { display: "contents" },
        title: { fontWeight: 600 },
        body: { gap: "4px" },
      }}
    >
      {expanded && (
        <Text fz={11} id={descriptionId}>
          {description}
        </Text>
      )}
    </Alert>
  );
}
