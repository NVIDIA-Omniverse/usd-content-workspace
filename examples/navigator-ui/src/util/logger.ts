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

/**
 * Diagnostic logging for developers, written to the browser console.
 *
 * Use this instead of calling `console` directly: it keeps the `no-console` exemption in one
 * place and gives a single seam for routing diagnostics elsewhere later. User-facing messages
 * belong in `notificationService` instead.
 */
export const logger = {
  warn: (message: string, ...details: unknown[]) => {
    // eslint-disable-next-line no-console
    console.warn(message, ...details);
  },
  error: (message: string, ...details: unknown[]) => {
    // eslint-disable-next-line no-console
    console.error(message, ...details);
  },
};
