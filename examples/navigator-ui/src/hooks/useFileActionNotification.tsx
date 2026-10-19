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

import { useCallback, type CSSProperties } from "react";
import { IconAlertTriangleFilled, IconCancel, IconCheck, IconExclamationCircleFilled } from "@tabler/icons-react";
import { ActionIcon, Flex, Text } from "@mantine/core";
import { NotificationMessage } from "@/components/NotificationMessage";
import { ProgressPercentIcon } from "@/components/ProgressPercentIcon";
import { useNotification } from "@/hooks/useNotification";

export interface UseFileActionNotificationParams<T extends { name: string }> {
  action: string;
  data: T[];
}

export function useFileActionNotification<T extends { name: string }>() {
  const notifications = useNotification();

  return useCallback(
    ({ action, data }: UseFileActionNotificationParams<T>) => {
      const abortController = new AbortController();
      const handleCancel = () => {
        abortController.abort();
        notification.cancel();
      };

      const resource = data.length === 1 ? data[0].name : `${data.length} items`;
      const title = `${action} ${resource}`;

      const id = notifications.show({
        title,
        message: (
          <NotificationMessage onCancel={handleCancel}>
            <Text size="xs" c="dimmed">
              Starting...
            </Text>
          </NotificationMessage>
        ),
        loading: true,
        autoClose: false,
        withCloseButton: false,
      });

      const notification = {
        id,
        signal: abortController.signal,
        update: (text: string) => {
          notifications.update({
            id,
            message: (
              <NotificationMessage onCancel={handleCancel}>
                <Text size="xs" c="dimmed" flex={1} title={text} miw={0} style={ellipsisTextStyles}>
                  {text}
                </Text>
              </NotificationMessage>
            ),
          });
        },
        progress: (primaryMessage: string, secondaryMessage?: string, percent?: number) => {
          notifications.update({
            id,
            message: (
              <NotificationMessage onCancel={handleCancel}>
                <Flex justify="space-between" gap="xs" align="center" flex={1} w="100%">
                  <Text size="xs" c="dimmed" flex={1} miw={0} title={primaryMessage} style={ellipsisTextStyles}>
                    {primaryMessage}
                  </Text>
                  {secondaryMessage && (
                    <Text size="xs" c="dimmed" style={{ whiteSpace: "nowrap" }}>
                      {secondaryMessage}
                    </Text>
                  )}
                </Flex>
              </NotificationMessage>
            ),
            icon: percent !== undefined ? <ProgressPercentIcon percent={percent} /> : undefined,
            loading: percent === undefined,
          });
        },
        cancel: () => {
          notifications.update({
            id,
            title,
            loading: false,
            color: "dark",
            icon: <IconCancel size={22} />,
            message: "Cancelled.",
            autoClose: document.hasFocus() ? 5000 : false,
            withCloseButton: true,
          });
        },
        complete: <T,>(completed: T[], skipped: string[] = []) => {
          const message =
            skipped.length === 0
              ? completed.length === 1
                ? "Success."
                : `Success: ${completed.length} items.`
              : `Success: ${completed.length} completed, ${skipped.length} skipped.`;

          notifications.update({
            id,
            title,
            loading: false,
            color: "green",
            icon: <IconCheck size={22} />,
            message: <Text size="xs">{message}</Text>,
            autoClose: document.hasFocus() ? 5000 : false,
            withCloseButton: true,
          });
        },
        completePartially: <T,>(completed: T[], failed: T[], skipped: string[] = []) => {
          const parts = [`${completed.length} completed`];
          if (skipped.length > 0) {
            parts.push(`${skipped.length} skipped`);
          }
          parts.push(`${failed.length} failed`);

          notifications.update({
            id,
            title,
            loading: false,
            icon: (
              <ActionIcon c="yellow" variant="transparent">
                <IconAlertTriangleFilled color="orange" size={22} />
              </ActionIcon>
            ),
            message: <Text size="xs">Completed partially: {parts.join(", ")}.</Text>,
            withCloseButton: true,
          });
        },
        fail: <T,>(failed: T[]) => {
          notifications.update({
            id,
            title,
            loading: false,
            icon: (
              <ActionIcon c="red" variant="transparent">
                <IconExclamationCircleFilled size={22} />
              </ActionIcon>
            ),
            message: (
              <Text size="xs">
                Failed to {action.toLowerCase()} {failed.length === 1 ? resource : `${failed.length} items`}.
              </Text>
            ),
            autoClose: document.hasFocus() ? 5000 : false,
            withCloseButton: true,
          });
        },
      };

      return notification;
    },
    [notifications]
  );
}

const ellipsisTextStyles: CSSProperties = {
  textOverflow: "ellipsis",
  textWrap: "wrap",
  wordBreak: "break-all",
  overflow: "hidden",
};
