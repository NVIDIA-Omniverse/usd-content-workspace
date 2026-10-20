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

import { Center, Loader } from "@mantine/core";

export interface PageLoaderProps {
  label?: string;
}

/**
 * Full-viewport centered spinner used to gate the entire app while a
 * top-level async dependency (config, default-server bootstrap, OIDC
 * exchange) is in flight. Keeping a single component for these states
 * prevents the spinner from "jumping" between transitions.
 */
export function PageLoader({ label = "Loading" }: PageLoaderProps) {
  return (
    <Center h="100vh" w="100vw">
      <Loader aria-label={label} />
    </Center>
  );
}
