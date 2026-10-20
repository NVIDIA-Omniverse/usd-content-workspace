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

Feature: IsVersioned() VersioningAPI

  Background:
    Given a new test namespace called 'is_versioned_test_alpha'
    Given a connection to the storage service
    And an authenticated user

  Scenario Outline: IsVersioned accepts a concrete nested resource address
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'versioning' API in version 'v1alpha' for feature 'versioning'
    And a resource address for 'nested/path/is-versioned-<protocol>.usd'
    Then calling '<protocol>' IsVersioned on that address returns '<return_value>' with a boolean is_versioned field
  Examples:
    | protocol | return_value |
    | GRPC     | OK           |
    | REST     | 200          |
