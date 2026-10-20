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

import { jwtDecode, type JwtPayload } from "jwt-decode";
import type { ServiceMetadata } from "../api/schemas.js";

export function getTokenPrincipal(
  idToken: string,
  metadata: ServiceMetadata,
): string {
  if (!idToken) {
    return "";
  }

  const payload = jwtDecode<JwtPayload & Record<string, unknown>>(idToken);
  const claim = payload[metadata.id_claim] ?? payload.sub;
  return typeof claim === "string" ? claim : "";
}
