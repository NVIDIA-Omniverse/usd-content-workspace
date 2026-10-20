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

import { RingProgress, Text } from "@mantine/core";

interface ProgressProps {
  percent: number;
}

export function ProgressPercentIcon({ percent }: ProgressProps) {
  return (
    <RingProgress
      label={
        <Text size="10px" ta="center">
          {percent.toFixed(0)}%
        </Text>
      }
      size={50}
      thickness={3}
      sections={[{ value: percent, color: "blue" }]}
    />
  );
}
