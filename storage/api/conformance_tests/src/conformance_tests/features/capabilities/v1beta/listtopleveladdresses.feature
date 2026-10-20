# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

Feature: ListTopLevelAddresses() CapabilitesAPI

  Background:
    Given a connection to the storage service
    And an authenticated user

Scenario Outline: Top-level addresses are discoverable and enumerable
  Given the service speaks '<protocol>'
  And the '<protocol>' service supports the 'capabilities' API in version 'v1beta' for feature 'capabilities'
  And the '<protocol>' service supports the 'fileobject' API in version 'v1beta' for feature 'fileobject'
  When calling '<protocol>' ListTopLevelAddresses returns '<return_value>'
  When we memorize the last response as 'top_level_addresses'
  Then calling '<protocol>' Enumerate on that topleveladdresses returns '<return_value>'
Examples:
  | protocol | return_value |
  | GRPC     | OK           |
  | REST     | 200          |