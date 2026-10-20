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

import { ReactNode } from "react";
import { IconAlertCircle, IconCheck, IconInfoCircle, IconProps, IconX } from "@tabler/icons-react";
import { act, renderHook } from "@testing-library/react";
import { notifications, NotificationsStore } from "@mantine/notifications";
import { NotificationProps, useNotification } from "@/hooks/useNotification";

// Mock the @mantine/notifications module
vi.mock("@mantine/notifications", () => ({
  notifications: {
    show: vi.fn(),
    hide: vi.fn(),
    update: vi.fn(),
    clean: vi.fn(),
    cleanQueue: vi.fn(),
  } as unknown as typeof notifications,
}));

describe("useNotification", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("should show a success notification with default props", () => {
    const { result } = renderHook(() => useNotification());

    act(() => {
      result.current.show({
        title: "Success",
        message: "Operation completed successfully",
        type: "success",
      });
    });

    expect(notifications.show).toHaveBeenCalledWith(
      expect.objectContaining<NotificationProps>({
        title: "Success",
        message: "Operation completed successfully",
        type: "success",
        color: "green",
        autoClose: 3_000,
        icon: expect.objectContaining({
          type: IconCheck,
          props: expect.objectContaining({
            title: "success",
          }) as IconProps,
        }) as ReactNode,
      }),
      undefined
    );
  });

  it("should show an info notification with default props", () => {
    const { result } = renderHook(() => useNotification());

    act(() => {
      result.current.show({
        title: "Info",
        message: "Here's some information",
        type: "info",
      });
    });

    expect(notifications.show).toHaveBeenCalledWith(
      expect.objectContaining<NotificationProps>({
        title: "Info",
        message: "Here's some information",
        type: "info",
        color: "blue",
        autoClose: 3_000,
        icon: expect.objectContaining({
          type: IconInfoCircle,
          props: expect.objectContaining({
            title: "info",
          }) as IconProps,
        }) as ReactNode,
      }),
      undefined
    );
  });

  it("should show a warning notification with default props", () => {
    const { result } = renderHook(() => useNotification());

    act(() => {
      result.current.show({
        title: "Warning",
        message: "Please be careful",
        type: "warning",
      });
    });

    expect(notifications.show).toHaveBeenCalledWith(
      expect.objectContaining<NotificationProps>({
        title: "Warning",
        message: "Please be careful",
        type: "warning",
        color: "yellow",
        autoClose: 7_500,
        icon: expect.objectContaining({
          type: IconAlertCircle,
          props: expect.objectContaining({
            title: "warning",
          }) as IconProps,
        }) as ReactNode,
      }),
      undefined
    );
  });

  it("should show an error notification with default props", () => {
    const { result } = renderHook(() => useNotification());

    act(() => {
      result.current.show({
        title: "Error",
        message: "Something went wrong",
        type: "error",
      });
    });

    expect(notifications.show).toHaveBeenCalledWith(
      expect.objectContaining<NotificationProps>({
        title: "Error",
        message: "Something went wrong",
        type: "error",
        color: "red",
        autoClose: 7_500,
        icon: expect.objectContaining({
          type: IconX,
          props: expect.objectContaining({
            title: "error",
          }) as IconProps,
        }) as ReactNode,
      }),
      undefined
    );
  });

  it("should show a custom notification without type", () => {
    const { result } = renderHook(() => useNotification());

    act(() => {
      result.current.show({
        title: "Custom",
        message: "Custom notification",
        color: "purple",
      });
    });

    expect(notifications.show).toHaveBeenCalledWith(
      expect.objectContaining<NotificationProps>({
        title: "Custom",
        message: "Custom notification",
        color: "purple",
      }),
      undefined
    );
  });

  it("should override default props with custom props", () => {
    const { result } = renderHook(() => useNotification());

    act(() => {
      result.current.show({
        title: "Custom Success",
        message: "Custom success message",
        type: "success",
        color: "teal",
        autoClose: 10_000,
      });
    });

    expect(notifications.show).toHaveBeenCalledWith(
      expect.objectContaining<NotificationProps>({
        title: "Custom Success",
        message: "Custom success message",
        type: "success",
        color: "teal",
        autoClose: 10_000,
        icon: expect.objectContaining({
          type: IconCheck,
          props: expect.objectContaining({
            title: "success",
          }) as IconProps,
        }) as ReactNode,
      }),
      undefined
    );
  });

  it("should allow specifying a custom notification store", () => {
    const { result } = renderHook(() => useNotification());
    const customStore = {} as NotificationsStore;

    act(() => {
      result.current.show(
        {
          title: "Custom Store",
          message: "Using custom store",
          type: "info",
        },
        customStore
      );
    });

    expect(notifications.show).toHaveBeenCalledWith(
      expect.objectContaining<NotificationProps>({
        title: "Custom Store",
        message: "Using custom store",
        type: "info",
      }) as NotificationProps,
      customStore
    );
  });
});
