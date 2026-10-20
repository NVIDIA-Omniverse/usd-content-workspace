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

import { createInterface } from "readline";
import { type Readable } from "stream";

// CSI sequences: ESC [ ... <final byte in 0x40-0x7E>
// OSC sequences: ESC ] ... (BEL or ESC \)
// eslint-disable-next-line no-control-regex
const ANSI_RE = /\x1b(?:\[[0-9;]*[A-Za-z]|\][^\x07\x1b]*(?:\x07|\x1b\\))/g;

export function isStdinPiped(): boolean {
  return !process.stdin.isTTY;
}

/** Strips ANSI codes, trims whitespace, and filters blank lines from each input line. */
export async function readStdinLines(
  input: Readable = process.stdin,
): Promise<string[]> {
  const rl = createInterface({ input, terminal: false });
  const lines: string[] = [];
  for await (const raw of rl) {
    const line = raw.replace(ANSI_RE, "").trim();
    if (line) lines.push(line);
  }
  return lines;
}
