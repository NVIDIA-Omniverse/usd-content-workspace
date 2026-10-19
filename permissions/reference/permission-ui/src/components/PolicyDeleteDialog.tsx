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
import { CodeHighlight } from "@mantine/code-highlight";
import {
  Alert,
  Button,
  Group,
  Loader,
  Paper,
  ScrollArea,
  Stack,
  Text,
  Tooltip,
} from "@mantine/core";
import { notifications } from "@mantine/notifications";
import {
  IconCircleCheckFilled,
  IconCircleXFilled,
  IconTrashFilled,
} from "@tabler/icons-react";
import { useQueryClient } from "@tanstack/react-query";
import dedent from "dedent";
import { type ReactNode, useState } from "react";
import { useAuth } from "react-oidc-context";
import { useConfig } from "../hooks/useConfig";
import { deletePolicy, type Policy } from "../state/policies";
import { PolicyBadge } from "./PolicyBadge";

export interface PolicyDeleteDialogProps {
  policies: Policy[];
  onClose: () => void;
  onPoliciesDeleted: (ids: number[]) => void;
}

export function PolicyDeleteDialog({
  policies,
  onClose,
  onPoliciesDeleted,
}: PolicyDeleteDialogProps) {
  const config = useConfig();
  const auth = useAuth();
  const queryClient = useQueryClient();

  const [statuses, setStatuses] = useState<Record<number, PolicyDeleteStatus>>(
    () =>
      Object.fromEntries(
        policies.map((policy) => [policy.id, { state: "idle" }]),
      ),
  );
  const [isDeleting, setIsDeleting] = useState(false);

  const allDeleted = policies.every(
    (policy) => statuses[policy.id]?.state === "success",
  );
  const hasFailures = policies.some(
    (policy) => statuses[policy.id]?.state === "error",
  );

  const deletePolicies = async () => {
    const targets = policies.filter(
      (policy) => statuses[policy.id]?.state !== "success",
    );

    setIsDeleting(true);
    setStatuses((prev) => {
      const next = { ...prev };
      for (const policy of targets) {
        next[policy.id] = { state: "deleting" };
      }
      return next;
    });

    const results = await Promise.all(
      targets.map(async (policy) => {
        try {
          await deletePolicy({ policyId: policy.id, config, user: auth.user! });
          return { id: policy.id, ok: true as const };
        } catch (error) {
          return {
            id: policy.id,
            ok: false as const,
            error: error instanceof Error ? error.message : String(error),
          };
        }
      }),
    );

    setStatuses((prev) => {
      const next = { ...prev };
      for (const result of results) {
        next[result.id] = result.ok
          ? { state: "success" }
          : { state: "error", error: result.error };
      }
      return next;
    });
    setIsDeleting(false);

    const deletedIds = results.filter((r) => r.ok).map((r) => r.id);
    const failedCount = results.length - deletedIds.length;

    if (deletedIds.length > 0) {
      onPoliciesDeleted(deletedIds);
      void queryClient.invalidateQueries({ queryKey: ["policies"] });
    }

    if (failedCount === 0) {
      notifications.show({
        title: `Deleted ${deletedIds.length} ${deletedIds.length === 1 ? "policy" : "policies"}.`,
        message: "",
        icon: <IconTrashFilled />,
        color: "green",
      });
    } else {
      notifications.show({
        title: `Failed to delete ${failedCount} ${failedCount === 1 ? "policy" : "policies"}.`,
        message: "",
        color: "red",
      });
    }
  };

  return (
    <Stack gap={"sm"}>
      <Text size={"sm"}>
        {allDeleted
          ? "The following policies have been deleted."
          : `Are you sure you want to delete the following ${policies.length === 1 ? "policy" : `${policies.length} policies`}?`}
      </Text>

      <ScrollArea.Autosize mah={360} type={"auto"} scrollbars={"y"}>
        <Stack gap={"sm"} pr={"md"}>
          {policies.map((policy) => (
            <PolicyDeleteCard
              key={policy.id}
              policy={policy}
              status={statuses[policy.id]}
            />
          ))}
        </Stack>
      </ScrollArea.Autosize>

      {!allDeleted ? (
        <Text size={"sm"} fw={500} c={"red"}>
          This action cannot be undone.
        </Text>
      ) : null}

      <Group justify={"flex-end"} mt={"sm"}>
        <Button variant={"default"} onClick={onClose} disabled={isDeleting}>
          {allDeleted ? "Close" : "Cancel"}
        </Button>
        {allDeleted ? null : (
          <Button color={"red"} loading={isDeleting} onClick={deletePolicies}>
            {hasFailures ? "Retry" : "Delete"}
          </Button>
        )}
      </Group>
    </Stack>
  );
}

type PolicyDeleteState = "idle" | "deleting" | "success" | "error";

interface PolicyDeleteStatus {
  state: PolicyDeleteState;
  error?: string;
}

function PolicyDeleteCard({
  policy,
  status,
}: {
  policy: Policy;
  status?: PolicyDeleteStatus;
}) {
  return (
    <Paper withBorder radius={"md"} p={"sm"}>
      <Group justify={"space-between"} wrap={"nowrap"} mb={"sm"}>
        <Group gap={"xs"}>
          <Text fw={600} size={"sm"}>
            #{policy.id}
          </Text>
          <PolicyBadge.Effect policy={policy} />
        </Group>
        <PolicyDeleteStatusIcon policy={policy} status={status} />
      </Group>

      <Group gap={"lg"} align={"flex-start"} grow mb={"sm"} wrap={"wrap"}>
        <PolicyDeleteField label={"Principal"}>
          <PolicyBadge.Principal policy={policy} />
        </PolicyDeleteField>
        <PolicyDeleteField label={"Action"}>
          {policy.action
            ? `${policy.action.service}:${policy.action.name}`
            : "Any"}
        </PolicyDeleteField>
        <PolicyDeleteField label={"Resource"}>
          {policy.resource
            ? `${policy.resource.type}:${policy.resource.id}`
            : "Any"}
        </PolicyDeleteField>
      </Group>

      <CodeHighlight
        code={dedent(policy.policy)}
        language={"cedar"}
        withBorder
        styles={{
          pre: { whiteSpace: "pre-wrap", wordBreak: "break-word" },
          code: { whiteSpace: "pre-wrap" },
        }}
      />

      {status?.state === "error" ? (
        <Alert
          color={"red"}
          mt={"sm"}
          title={`Failed to delete policy #${policy.id}`}
        >
          {status.error}
        </Alert>
      ) : null}

      {policy.comment ? (
        <Text size={"xs"} c={"dimmed"} mt={"xs"}>
          {policy.comment}
        </Text>
      ) : null}
    </Paper>
  );
}

function PolicyDeleteStatusIcon({
  policy,
  status,
}: {
  policy: Policy;
  status?: PolicyDeleteStatus;
}) {
  if (!status || status.state === "idle") {
    return null;
  }

  if (status.state === "deleting") {
    return <Loader size={"sm"} aria-label={`Deleting policy #${policy.id}`} />;
  }

  if (status.state === "success") {
    return (
      <Tooltip label={"Deleted"} withArrow>
        <Text
          component={"span"}
          c={"green"}
          aria-label={`Policy #${policy.id} deleted`}
        >
          <IconCircleCheckFilled size={22} />
        </Text>
      </Tooltip>
    );
  }

  return (
    <Tooltip label={"Failed to delete"} withArrow>
      <Text
        component={"span"}
        c={"red"}
        aria-label={`Policy #${policy.id} failed to delete`}
      >
        <IconCircleXFilled size={22} />
      </Text>
    </Tooltip>
  );
}

function PolicyDeleteField({
  label,
  children,
}: {
  label: string;
  children: ReactNode;
}) {
  return (
    <Stack gap={2} miw={120} style={{ flexBasis: 120 }}>
      <Text size={"xs"} c={"dimmed"} tt={"uppercase"} fw={600}>
        {label}
      </Text>
      <Text size={"sm"} style={{ wordBreak: "break-all" }} component={"div"}>
        {children}
      </Text>
    </Stack>
  );
}
