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
import { Badge, Tooltip } from "@mantine/core";
import { getPolicyEffect, type Policy } from "../state/policies";
import { GroupName } from "./GroupName";
import { PrincipalName } from "./PrincipalName";

export interface PolicyBadgeProps {
  policy?: Policy;
}

export const PolicyBadge = {
  Effect: function PolicyEffect({ policy }: PolicyBadgeProps) {
    return policy && getPolicyEffect(policy) === "permit" ? (
      <Tooltip
        label={"This policy allows users to perform the service action."}
        withArrow
      >
        <Badge color={"green"}>Permit</Badge>
      </Tooltip>
    ) : (
      <Tooltip
        label={"This policy forbids users to perform the service action."}
        withArrow
      >
        <Badge color={"red"}>Forbid</Badge>
      </Tooltip>
    );
  },

  Principal: function PolicyPrincipal({ policy }: PolicyBadgeProps) {
    const sub = policy?.principal?.sub;
    const groupId = policy?.group?.id;

    if (groupId) {
      return <GroupName id={groupId} />;
    }

    if (!sub) {
      return (
        <Tooltip label={"This policy is applied for all users."} withArrow>
          <Badge color={"blue"} variant={"light"}>
            Unscoped
          </Badge>
        </Tooltip>
      );
    }

    return <PrincipalName sub={sub} />;
  },

  Service: function PolicyService({ policy }: PolicyBadgeProps) {
    return (
      policy?.action?.service ?? (
        <Tooltip
          label={"This policy is applied for all services and actions."}
          withArrow
        >
          <Badge color={"blue"} variant={"light"}>
            Unscoped
          </Badge>
        </Tooltip>
      )
    );
  },

  Resource: function PolicyResource({ policy }: PolicyBadgeProps) {
    return (
      policy?.resource?.type ?? (
        <Tooltip label={"This policy is applied for all resources."} withArrow>
          <Badge color={"blue"} variant={"light"}>
            Unscoped
          </Badge>
        </Tooltip>
      )
    );
  },
};
