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

import { format, isValid, parseISO } from "date-fns";

const HAS_TIMEZONE = /(?:[zZ]|[+-]\d{2}(?::?\d{2})?)$/;
const HAS_TIME_COMPONENT = /[Tt]\d{2}/;

export function parseStorageTimestamp(timestamp: string): Date | null {
  // Treat bare timestamps (no timezone designator) as UTC so format() renders in local time.
  // Guard with HAS_TIME_COMPONENT so bare dates like "2024-01-01" (whose "-01" tail would
  // otherwise satisfy HAS_TIMEZONE) are still treated as UTC.
  const hasTimezone = HAS_TIME_COMPONENT.test(timestamp) && HAS_TIMEZONE.test(timestamp);
  const utc = hasTimezone ? timestamp : `${timestamp}Z`;
  const date = parseISO(utc);

  return isValid(date) ? date : null;
}

export function formatDate(timestamp?: string): string {
  if (!timestamp) {
    return "";
  }
  const date = parseStorageTimestamp(timestamp);

  return date ? format(date, "MM/dd/yyyy HH:mm:ss") : "";
}
