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
import "@testing-library/jest-dom";
import { cleanup } from "@testing-library/react";
import { notifications } from "@mantine/notifications";
import { afterEach } from "vitest";
import "./util/tests/router";
import "./util/tests/auth";

window.HTMLElement.prototype.scrollIntoView = () => {};

// Mock matchMedia for jsdom compatibility. Reduced motion is forced on so
// Mantine renders Collapse/Transition content synchronously in tests.
Object.defineProperty(window, "matchMedia", {
  writable: true,
  value: vi.fn().mockImplementation((query) => ({
    matches: query.includes("prefers-reduced-motion"),
    media: query,
    onchange: null,
    addListener: vi.fn(),
    removeListener: vi.fn(),
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    dispatchEvent: vi.fn(),
  })),
});

Object.defineProperty(window, "ResizeObserver", {
  writable: true,
  value: class ResizeObserver {
    callback: () => void;

    constructor(callback: () => void) {
      this.callback = callback;
    }

    observe() {}
    disconnect() {}
    unobserve() {}
  },
});

Object.defineProperty(document, "fonts", {
  writable: true,
  value: {
    ready: Promise.resolve(),
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
  },
});

afterEach(() => {
  cleanup();
  notifications.clean();
  notifications.cleanQueue();
});
