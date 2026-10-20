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

import "@testing-library/jest-dom/vitest";

import { vi } from "vitest";

const { getComputedStyle } = window;
window.getComputedStyle = (elt) => getComputedStyle(elt);
window.HTMLElement.prototype.scrollIntoView = () => {};

global.matchMedia =
  global.matchMedia ||
  function (query) {
    return {
      matches: true,
      media: query,
      onchange: null,
      addListener: vi.fn(), // deprecated
      removeListener: vi.fn(), // deprecated
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      dispatchEvent: vi.fn(),
    };
  };

class ResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}

window.ResizeObserver = ResizeObserver;

// Web Lock API is not available in jsdom; mock it globally for Batch and other usages
if (typeof navigator !== "undefined" && !navigator.locks) {
  Object.defineProperty(navigator, "locks", {
    value: {
      request: async (_name, callback) => callback(),
    },
    configurable: true,
    writable: true,
  });
}

// Polyfill URL.createObjectURL and revokeObjectURL for tests
if (!URL.createObjectURL) {
  // eslint-disable-next-line @typescript-eslint/ban-ts-comment
  // @ts-ignore
  URL.createObjectURL = () => "blob:mock";
}
if (!URL.revokeObjectURL) {
  // @ts-ignore
  URL.revokeObjectURL = () => {};
}

vi.mock("zustand"); // to make it work like Jest (auto-mocking)
