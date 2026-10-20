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

import { IconAlertCircle, IconCheck, IconInfoCircle, IconX } from "@tabler/icons-react";
import { NotificationData, notifications, NotificationsStore } from "@mantine/notifications";

/**
 * Provides a wrapper around the Mantine notifications API.
 */
export function useNotification() {
  return notificationsModule;
}

const notificationsModule = {
  ...notifications,
  show: (notification: NotificationProps, store?: NotificationsStore) => {
    const notificationProps = notification.type
      ? {
          ...defaultNotificationProps[notification.type],
          ...notification,
        }
      : notification;

    return notifications.show(notificationProps, store);
  },
};

export type NotificationType = "success" | "info" | "warning" | "error";

export interface NotificationProps extends NotificationData {
  type?: NotificationType;
}

const defaultNotificationProps: Record<string, Partial<NotificationData>> = {
  ["success"]: {
    color: "green",
    icon: <IconCheck role="graphics-symbol" title="success" />,
    autoClose: 3_000,
  },
  ["info"]: {
    color: "blue",
    icon: <IconInfoCircle role="graphics-symbol" title="info" />,
    autoClose: 3_000,
  },
  ["warning"]: {
    color: "yellow",
    icon: <IconAlertCircle role="graphics-symbol" title="warning" />,
    autoClose: 7_500,
  },
  ["error"]: {
    color: "red",
    icon: <IconX role="graphics-symbol" title="error" />,
    autoClose: 7_500,
  },
};
