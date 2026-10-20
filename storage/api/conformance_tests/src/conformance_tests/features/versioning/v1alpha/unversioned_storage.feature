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

@unversioned
Feature: Unversioned storage behavior VersioningAPI

  These scenarios require the example service running with unversioned storage
  (FILESERVICE_VERSIONING_MODE=unversioned): a single current version, hard deletes,
  and identity reads of deleted objects that fail with NOT_FOUND.

  Background:
    Given a new test namespace called 'unversioned_test_alpha'
    And a connection to the storage service
    And an authenticated user

  Scenario Outline: IsVersioned reports false on unversioned storage
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'versioning' API in version 'v1alpha' for feature 'versioning'
    And a resource address for 'nested/path/unversioned-is-versioned-<protocol>.usd'
    Then calling '<protocol>' IsVersioned on that address returns '<return_value>' with is_versioned 'false'
    Examples:
      | protocol | return_value |
      | GRPC     | OK           |
      | REST     | 200          |

  Scenario Outline: EnumerateVersions returns only the current version on unversioned storage
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    And the '<protocol>' service supports the 'versioning' API in version 'v1alpha' for feature 'versioning'
    Given a resource address for 'unversioned-enumerate-<protocol>.txt'
    And an object of size '1024' exists at that address and is readable and has '3' versions
    When calling '<protocol>' EnumerateVersions exhaustively on that address returns '<status_code>'
    Then the EnumerateVersions returned '1' items
    Examples:
      | protocol | status_code |
      | GRPC     | OK          |
      | REST     | 200         |

  Scenario Outline: Delete is idempotent on unversioned storage
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    Given a resource address for 'unversioned-delete-<protocol>.txt'
    And an object of size '1024' exists at that address and is readable
    When calling '<protocol>' delete on that address returns '<delete_return_value>'
    Then calling '<protocol>' delete on that address returns '<delete_return_value>'
    And calling '<protocol>' stat on that address returns '<not_found_value>'
    Examples:
      | protocol | delete_return_value | not_found_value |
      | GRPC     | OK                  | NOT_FOUND       |
      | REST     | 204                 | 404             |

  # Overwriting through the service must retain no history: the superseded object identity is gone
  # from storage, so a read of it fails with NOT_FOUND rather than returning the stale prior content.
  Scenario Outline: A superseded identity is not readable after overwrite on unversioned storage
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    Given a resource address for 'unversioned-overwrite-<protocol>.txt'
    And no object exists at that address
    And a 'small' blob according to the upload options for that address, using '<protocol>'
    When calling '<protocol>' write on that address with data returns a created response
    And memorizing that response as 'first_version'
    And a 'small' blob according to the upload options for that address, using '<protocol>'
    And calling '<protocol>' write on that address with data returns a created response
    When calling '<protocol>' Read on the memorized response 'first_version' returns '<not_found_value>'
    Examples:
      | protocol | not_found_value |
      | GRPC     | NOT_FOUND       |
      | REST     | 404             |

  Scenario Outline: A redirect completion identity is not readable after overwrite on unversioned storage
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    Given a resource address for 'unversioned-redirect-overwrite-<protocol>.txt'
    And no object exists at that address
    And a 'medium' blob according to the upload options for that address, using '<protocol>', with rand seed '1084907'
    When calling '<protocol>' write on that address returns a redirect response
    Then uploading a file following a redirect succeeds
    When calling '<protocol>' complete redirect upload returns '<complete_return_value>'
    And we memorize the last response as 'redirect_completion'
    And a 'small' blob according to the upload options for that address, using '<protocol>', with rand seed '1084908'
    When calling '<protocol>' write on that address with data returns a created response
    When calling '<protocol>' Read on the memorized response 'redirect_completion' returns '<not_found_value>'

    Examples:
      | protocol | complete_return_value | not_found_value |
      | GRPC     | OK                    | NOT_FOUND       |
      | REST     | 200                   | 404             |

  Scenario Outline: A Copy identity is not readable after overwrite on unversioned storage
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    Given a resource address for 'unversioned-copy-source-<protocol>.txt'
    And a test object of size '1024' with rand seed '1084909' at that address
    And determining head resource identity with '<protocol>'
    And memorizing that resource identity as 'source_identity'
    Given a resource address for 'unversioned-copy-destination-<protocol>.txt'
    And memorizing that resource address as 'destination_address'
    When calling '<protocol>' copy from memorized 'source_identity' to memorized 'destination_address' returns '<copy_return_value>'
    And we memorize the last response as 'copy_response'
    And a 'small' blob according to the upload options for that address, using '<protocol>', with rand seed '1084910'
    When calling '<protocol>' write on that address with data returns a created response
    When calling '<protocol>' Read on the memorized response 'copy_response' returns '<not_found_value>'

    Examples:
      | protocol | copy_return_value | not_found_value |
      | GRPC     | OK                | NOT_FOUND       |
      | REST     | 201               | 404             |

  # After a hard delete, a well-formed identity of the removed object is indistinguishable from one
  # that never existed, so an identity read returns NOT_FOUND (agreed contract for unversioned storage).
  Scenario Outline: A deleted identity is not readable on unversioned storage
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    Given a resource address for 'unversioned-deleted-identity-<protocol>.txt'
    And no object exists at that address
    And a 'small' blob according to the upload options for that address, using '<protocol>'
    When calling '<protocol>' write on that address with data returns a created response
    And memorizing that response as 'deleted_version'
    And calling '<protocol>' delete on that address returns '<delete_return_value>'
    When calling '<protocol>' Read on the memorized response 'deleted_version' returns '<not_found_value>'
    Examples:
      | protocol | delete_return_value | not_found_value |
      | GRPC     | OK                  | NOT_FOUND       |
      | REST     | 204                 | 404             |
